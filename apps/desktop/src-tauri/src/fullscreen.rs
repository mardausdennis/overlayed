use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{constants::MAIN_WINDOW_NAME, hotkeys};

#[derive(Default)]
pub struct FullscreenState {
  enabled: AtomicBool,
  supported: AtomicBool,
  hidden: AtomicBool,
  #[cfg(target_os = "windows")]
  tracker: std::sync::Mutex<Option<platform::Tracker>>,
}

#[derive(Clone, Copy, Serialize)]
pub struct FullscreenStatus {
  enabled: bool,
  supported: bool,
  hidden: bool,
}

fn status(state: &FullscreenState) -> FullscreenStatus {
  FullscreenStatus {
    enabled: state.enabled.load(Ordering::Acquire),
    supported: state.supported.load(Ordering::Acquire),
    hidden: state.hidden.load(Ordering::Acquire),
  }
}

#[tauri::command]
pub fn get_fullscreen_state(state: State<'_, FullscreenState>) -> FullscreenStatus {
  status(&state)
}

#[tauri::command]
pub async fn set_fullscreen_tracking(
  window: WebviewWindow,
  enabled: bool,
) -> Result<FullscreenStatus, String> {
  if window.label() != MAIN_WINDOW_NAME {
    return Err("Only the overlay window configures fullscreen tracking".to_string());
  }
  let app = window.app_handle().clone();
  let handle = app.clone();
  let (send, receive) = tokio::sync::oneshot::channel();
  app
    .run_on_main_thread(move || {
      let state = handle.state::<FullscreenState>();
      state.enabled.store(
        enabled && state.supported.load(Ordering::Acquire),
        Ordering::Release,
      );
      evaluate(&handle);
      let _ = send.send(status(&state));
    })
    .map_err(|error| error.to_string())?;
  receive.await.map_err(|error| error.to_string())
}

fn evaluate(app: &AppHandle) {
  let state = app.state::<FullscreenState>();
  let hidden = state.enabled.load(Ordering::Acquire) && platform::foreground_is_fullscreen(app);
  let Some(window) = app.get_webview_window(MAIN_WINDOW_NAME) else {
    return;
  };
  match hotkeys::set_fullscreen_hidden(&window, hidden) {
    Ok(_) => {
      state.hidden.store(hidden, Ordering::Release);
    }
    Err(error) => log::warn!("Could not apply fullscreen visibility: {error}"),
  }
}

pub fn refresh(app: &AppHandle) {
  let handle = app.clone();
  if let Err(error) = app.run_on_main_thread(move || evaluate(&handle)) {
    log::warn!("Could not reevaluate fullscreen visibility: {error}");
  }
}

pub fn start(app: &AppHandle) {
  #[cfg(target_os = "windows")]
  match platform::start(app.clone()) {
    Ok(tracker) => {
      let state = app.state::<FullscreenState>();
      match state.tracker.lock() {
        Ok(mut owner) => {
          *owner = Some(tracker);
          state.supported.store(true, Ordering::Release);
        }
        Err(error) => {
          log::warn!("Could not own fullscreen tracking: {error}");
          platform::stop(tracker);
        }
      };
    }
    Err(error) => log::warn!("Fullscreen tracking is unavailable: {error}"),
  }
  #[cfg(not(target_os = "windows"))]
  let _ = app;
}

pub fn stop(app: &AppHandle) {
  #[cfg(target_os = "windows")]
  match app.state::<FullscreenState>().tracker.lock() {
    Ok(mut owner) => {
      if let Some(tracker) = owner.take() {
        platform::stop(tracker);
      }
    }
    Err(error) => log::warn!("Could not stop fullscreen tracking: {error}"),
  }
  #[cfg(not(target_os = "windows"))]
  let _ = app;
}

#[cfg(any(target_os = "windows", test))]
mod decision {
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
  }

  pub struct Foreground {
    pub rect: Rect,
    pub monitor: Rect,
    pub work_area: Rect,
    pub same_monitor: bool,
    pub own: bool,
    pub shell: bool,
    pub visible: bool,
    pub minimized: bool,
    pub cloaked: bool,
    pub maximized: bool,
  }

  fn near(a: i32, b: i32) -> bool {
    (i64::from(a) - i64::from(b)).abs() <= 2
  }

  pub fn is_fullscreen(window: &Foreground) -> bool {
    if !window.same_monitor
      || window.own
      || window.shell
      || !window.visible
      || window.minimized
      || window.cloaked
    {
      return false;
    }
    let r = window.rect;
    let m = window.monitor;
    let w = window.work_area;
    if r.right <= r.left || r.bottom <= r.top || m.right <= m.left || m.bottom <= m.top {
      return false;
    }
    // A maximized work-area window remains normal even with a very thin taskbar.
    if window.maximized
      && w != m
      && near(r.left, w.left)
      && near(r.top, w.top)
      && near(r.right, w.right)
      && near(r.bottom, w.bottom)
    {
      return false;
    }
    i64::from(r.left) <= i64::from(m.left) + 2
      && i64::from(r.top) <= i64::from(m.top) + 2
      && i64::from(r.right) >= i64::from(m.right) - 2
      && i64::from(r.bottom) >= i64::from(m.bottom) - 2
  }
}

#[cfg(not(target_os = "windows"))]
mod platform {
  pub fn foreground_is_fullscreen(_: &tauri::AppHandle) -> bool {
    false
  }
}

#[cfg(any(target_os = "windows", test))]
mod scheduling {
  #[derive(Clone, Copy)]
  pub enum Event {
    Foreground,
    Focus,
    CaptureStart,
    Timer,
    Other,
  }

  pub type Rechecks = [Option<u64>; 3];

  // Deadlines are relative to the hook thread's monotonic clock. Activation
  // can complete after its event; newer activation replaces the whole sequence.
  pub fn schedule(event: Event, now_ms: u64, pending: Rechecks) -> Rechecks {
    match event {
      Event::Foreground | Event::Focus | Event::CaptureStart => {
        [150, 500, 1500].map(|delay| Some(now_ms.saturating_add(delay)))
      }
      Event::Timer => pending.map(|deadline| deadline.filter(|deadline| *deadline > now_ms)),
      Event::Other => pending,
    }
  }
}

#[cfg(target_os = "windows")]
mod platform {
  use super::decision::{self, Foreground, Rect};
  use super::scheduling::{self, Event, Rechecks};
  use std::{cell::RefCell, mem::size_of, ptr::null_mut, sync::mpsc, thread, time::Instant};
  use tauri::{AppHandle, Manager};
  use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
      Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
      Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    },
    System::Threading::GetCurrentThreadId,
    UI::{
      Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
      WindowsAndMessaging::*,
    },
  };

  const RECHECK: u32 = WM_APP + 41;
  pub struct Tracker {
    thread_id: u32,
    thread: thread::JoinHandle<()>,
  }
  struct Context {
    app: AppHandle,
    overlay: usize,
    queued: bool,
    clock: Instant,
    rechecks: Rechecks,
    timer: Option<usize>,
  }
  thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }

  impl Context {
    fn now_ms(&self) -> u64 {
      self.clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    fn queue_recheck(&mut self) {
      if self.queued {
        return;
      }
      // SAFETY: called only on the hook thread after its message queue exists.
      if unsafe { PostThreadMessageW(GetCurrentThreadId(), RECHECK, 0, 0) } != 0 {
        self.queued = true;
      } else {
        log::warn!(
          "Could not schedule fullscreen check: {}",
          std::io::Error::last_os_error()
        );
      }
    }

    fn cancel_timer(&mut self) {
      if let Some(timer) = self.timer.take() {
        // SAFETY: this thread owns the null-hwnd timer ID returned by SetTimer.
        if unsafe { KillTimer(null_mut(), timer) } == 0 {
          log::warn!(
            "Could not remove fullscreen recheck timer: {}",
            std::io::Error::last_os_error()
          );
        }
      }
    }

    fn arm_timer(&mut self) {
      self.cancel_timer();
      let Some(deadline) = self.rechecks.iter().flatten().min().copied() else {
        return;
      };
      let delay = deadline
        .saturating_sub(self.now_ms())
        .clamp(1, u64::from(u32::MAX)) as u32;
      // SAFETY: thread timer, no callback; WM_TIMER is handled in our message loop.
      let timer = unsafe { SetTimer(null_mut(), 0, delay, None) };
      if timer == 0 {
        log::warn!(
          "Could not defer fullscreen check: {}",
          std::io::Error::last_os_error()
        );
      } else {
        self.timer = Some(timer);
      }
    }

    fn timer_elapsed(&mut self, id: usize) {
      if self.timer != Some(id) {
        return;
      }
      let pending = scheduling::schedule(Event::Timer, self.now_ms(), self.rechecks);
      // KillTimer does not remove queued messages. Checking the deadline also
      // rejects an old message if Windows reused its timer ID after a restart.
      if pending != self.rechecks {
        self.rechecks = pending;
        self.arm_timer();
        self.queue_recheck();
      }
    }
  }

  impl Drop for Context {
    fn drop(&mut self) {
      self.cancel_timer();
    }
  }

  struct Hooks(Vec<HWINEVENTHOOK>);
  impl Drop for Hooks {
    fn drop(&mut self) {
      for hook in self.0.drain(..) {
        // SAFETY: each hook is installed and removed on this message-loop thread.
        if unsafe { UnhookWinEvent(hook) } == 0 {
          log::warn!(
            "Could not remove fullscreen WinEvent hook: {}",
            std::io::Error::last_os_error()
          );
        }
      }
    }
  }

  unsafe extern "system" fn changed(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
  ) {
    CONTEXT.with(|context| {
      let Ok(mut context) = context.try_borrow_mut() else {
        return;
      };
      let Some(context) = context.as_mut() else {
        return;
      };
      let activation = match event {
        EVENT_SYSTEM_FOREGROUND => Event::Foreground,
        EVENT_OBJECT_FOCUS => Event::Focus,
        EVENT_SYSTEM_CAPTURESTART => Event::CaptureStart,
        _ => Event::Other,
      };
      let schedules_rechecks = !matches!(activation, Event::Other);
      if schedules_rechecks {
        context.rechecks = scheduling::schedule(activation, context.now_ms(), context.rechecks);
        context.arm_timer();
      }
      // Focus can belong to a child control and foreground can still be the
      // previous hwnd. Do not apply the geometry filter to activation events.
      let relevant = schedules_rechecks
        || (object == OBJID_WINDOW
          && child == 0
          && (hwnd == GetForegroundWindow() || hwnd as usize == context.overlay));
      if relevant {
        context.queue_recheck();
      }
    });
  }

  pub fn start(app: AppHandle) -> Result<Tracker, String> {
    let overlay = app
      .get_webview_window(crate::constants::MAIN_WINDOW_NAME)
      .ok_or_else(|| "The overlay window is unavailable".to_string())?
      .hwnd()
      .map_err(|e| e.to_string())?
      .0 as usize;
    let (send, receive) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
      .name("overlayed-fullscreen".to_string())
      .spawn(move || {
        // SAFETY: the queue, hooks, callbacks and removal all live on this thread.
        unsafe {
          let mut message: MSG = std::mem::zeroed();
          PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
          let mut hooks = Hooks(Vec::new());
          for event in [
            EVENT_SYSTEM_FOREGROUND,
            EVENT_OBJECT_FOCUS,
            EVENT_SYSTEM_CAPTURESTART,
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_SYSTEM_MOVESIZEEND,
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE,
            EVENT_OBJECT_CLOAKED,
            EVENT_OBJECT_UNCLOAKED,
            EVENT_OBJECT_DESTROY,
          ] {
            let hook = SetWinEventHook(
              event,
              event,
              null_mut(),
              Some(changed),
              0,
              0,
              WINEVENT_OUTOFCONTEXT,
            );
            if hook.is_null() {
              let _ = send.send(Err(std::io::Error::last_os_error().to_string()));
              return;
            }
            hooks.0.push(hook);
          }
          CONTEXT.with(|context| {
            *context.borrow_mut() = Some(Context {
              app,
              overlay,
              queued: false,
              clock: Instant::now(),
              rechecks: [None; 3],
              timer: None,
            })
          });
          if send.send(Ok(GetCurrentThreadId())).is_err() {
            return;
          }
          loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result == 0 {
              break;
            }
            if result == -1 {
              log::warn!(
                "Fullscreen message loop failed: {}",
                std::io::Error::last_os_error()
              );
              break;
            }
            if message.message == RECHECK {
              CONTEXT.with(|context| {
                if let Some(context) = context.borrow_mut().as_mut() {
                  context.queued = false;
                  super::refresh(&context.app);
                }
              });
            }
            if message.message == WM_TIMER {
              CONTEXT.with(|context| {
                if let Some(context) = context.borrow_mut().as_mut() {
                  context.timer_elapsed(message.wParam);
                }
              });
              continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
          }
          CONTEXT.with(|context| *context.borrow_mut() = None);
        }
      })
      .map_err(|e| e.to_string())?;
    match receive.recv() {
      Ok(Ok(thread_id)) => Ok(Tracker { thread_id, thread }),
      result => {
        let _ = thread.join();
        Err(format!("Could not start WinEvent hooks: {result:?}"))
      }
    }
  }

  pub fn stop(tracker: Tracker) {
    // SAFETY: thread ID comes from the live thread after queue creation.
    if unsafe { PostThreadMessageW(tracker.thread_id, WM_QUIT, 0, 0) } == 0 {
      log::warn!(
        "Could not stop fullscreen message loop: {}",
        std::io::Error::last_os_error()
      );
      return;
    }
    if tracker.thread.join().is_err() {
      log::warn!("Fullscreen message-loop thread failed during exit");
    }
  }

  fn rect(rect: RECT) -> Rect {
    Rect {
      left: rect.left,
      top: rect.top,
      right: rect.right,
      bottom: rect.bottom,
    }
  }

  pub fn foreground_is_fullscreen(app: &AppHandle) -> bool {
    let Some(overlay) = app.get_webview_window(crate::constants::MAIN_WINDOW_NAME) else {
      return false;
    };
    let Ok(overlay) = overlay.hwnd() else {
      return false;
    };
    // SAFETY: read-only Win32 snapshots; handles/rects are checked before use.
    unsafe {
      let hwnd = GetForegroundWindow();
      if hwnd.is_null() {
        return false;
      }
      let mut pid = 0;
      GetWindowThreadProcessId(hwnd, &mut pid);
      if pid == std::process::id() || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
        return false;
      }
      let mut class = [0u16; 256];
      let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
      let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
      let shell = hwnd == GetShellWindow()
        || hwnd == GetDesktopWindow()
        || class.eq_ignore_ascii_case("Progman")
        || class.eq_ignore_ascii_case("WorkerW");
      let mut cloaked: u32 = 0;
      if DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED as u32,
        &mut cloaked as *mut _ as _,
        size_of::<u32>() as u32,
      ) < 0
      {
        return false;
      }
      let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
      let overlay_monitor = MonitorFromWindow(overlay.0 as HWND, MONITOR_DEFAULTTONEAREST);
      if monitor.is_null() || overlay_monitor.is_null() {
        return false;
      }
      let mut info: MONITORINFO = std::mem::zeroed();
      info.cbSize = size_of::<MONITORINFO>() as u32;
      if GetMonitorInfoW(monitor, &mut info) == 0 {
        return false;
      }
      let mut bounds: RECT = std::mem::zeroed();
      if DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS as u32,
        &mut bounds as *mut _ as _,
        size_of::<RECT>() as u32,
      ) < 0
        && GetWindowRect(hwnd, &mut bounds) == 0
      {
        return false;
      }
      decision::is_fullscreen(&Foreground {
        rect: rect(bounds),
        monitor: rect(info.rcMonitor),
        work_area: rect(info.rcWork),
        same_monitor: monitor == overlay_monitor,
        own: false,
        shell,
        visible: true,
        minimized: false,
        cloaked: cloaked != 0,
        maximized: IsZoomed(hwnd) != 0,
      })
    }
  }
}

#[cfg(test)]
mod tests {
  use super::decision::*;
  use super::scheduling::{schedule, Event};

  #[test]
  fn activation_events_schedule_three_bounded_rechecks() {
    for event in [Event::Foreground, Event::Focus, Event::CaptureStart] {
      assert_eq!(
        schedule(event, 100, [None; 3]),
        [Some(250), Some(600), Some(1600)]
      );
    }
  }

  #[test]
  fn newer_activation_restarts_the_entire_sequence() {
    let pending = schedule(Event::Foreground, 0, [None; 3]);
    let pending = schedule(Event::Timer, 150, pending);
    assert_eq!(pending, [None, Some(500), Some(1500)]);
    for event in [Event::Foreground, Event::Focus, Event::CaptureStart] {
      assert_eq!(
        schedule(event, 300, pending),
        [Some(450), Some(800), Some(1800)]
      );
    }
  }

  #[test]
  fn timer_sequence_finishes_and_idle_never_schedules() {
    let pending = schedule(Event::Foreground, 100, [None; 3]);
    assert_eq!(schedule(Event::Timer, 249, pending), pending);
    let pending = schedule(Event::Timer, 250, pending);
    assert_eq!(pending, [None, Some(600), Some(1600)]);
    let pending = schedule(Event::Timer, 600, pending);
    assert_eq!(pending, [None, None, Some(1600)]);
    let pending = schedule(Event::Timer, 1600, pending);
    assert_eq!(pending, [None; 3]);
    assert_eq!(schedule(Event::Timer, 10_000, pending), [None; 3]);
    assert_eq!(schedule(Event::Other, 10_000, pending), [None; 3]);
  }

  #[test]
  fn geometry_events_do_not_extend_activation_rechecks() {
    let pending = schedule(Event::Foreground, 0, [None; 3]);
    assert_eq!(schedule(Event::Other, 100, pending), pending);
    assert_eq!(schedule(Event::Timer, 2000, pending), [None; 3]);
  }

  fn window() -> Foreground {
    let monitor = Rect {
      left: -1920,
      top: 0,
      right: 0,
      bottom: 1080,
    };
    Foreground {
      rect: monitor,
      monitor,
      work_area: Rect {
        bottom: 1040,
        ..monitor
      },
      same_monitor: true,
      own: false,
      shell: false,
      visible: true,
      minimized: false,
      cloaked: false,
      maximized: false,
    }
  }
  #[test]
  fn covers_the_whole_monitor_and_allows_two_pixel_tolerance() {
    let mut w = window();
    assert!(is_fullscreen(&w));
    w.rect = Rect {
      left: -1918,
      top: 2,
      right: -2,
      bottom: 1078,
    };
    assert!(is_fullscreen(&w));
    w.rect.bottom = 1077;
    assert!(!is_fullscreen(&w));
  }
  #[test]
  fn other_monitor_never_hides_the_overlay() {
    let mut w = window();
    w.same_monitor = false;
    assert!(!is_fullscreen(&w));
  }
  #[test]
  fn maximized_work_area_with_taskbar_is_not_fullscreen() {
    let mut w = window();
    w.maximized = true;
    w.rect = w.work_area;
    assert!(!is_fullscreen(&w));
    w.work_area.bottom = 1079;
    w.rect = w.work_area;
    assert!(!is_fullscreen(&w));
  }
  #[test]
  fn minimized_invisible_and_cloaked_windows_do_not_hide() {
    let mut w = window();
    w.minimized = true;
    assert!(!is_fullscreen(&w));
    w.minimized = false;
    w.visible = false;
    assert!(!is_fullscreen(&w));
    w.visible = true;
    w.cloaked = true;
    assert!(!is_fullscreen(&w));
  }
  #[test]
  fn own_windows_are_excluded() {
    let mut w = window();
    w.own = true;
    assert!(!is_fullscreen(&w));
  }
  #[test]
  fn shell_and_desktop_windows_are_excluded() {
    let mut w = window();
    w.shell = true;
    assert!(!is_fullscreen(&w));
  }
  #[test]
  fn invalid_and_partial_rectangles_are_not_fullscreen() {
    let mut w = window();
    w.rect.right = w.rect.left;
    assert!(!is_fullscreen(&w));
    w = window();
    w.rect.left += 500;
    assert!(!is_fullscreen(&w));
  }
}
