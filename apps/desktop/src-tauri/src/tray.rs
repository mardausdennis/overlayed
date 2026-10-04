use std::sync::Mutex;

use tauri::{
  menu::{Menu, MenuBuilder, MenuEvent},
  tray::TrayIconBuilder,
  AppHandle, Emitter, LogicalSize, Manager, Wry,
};

use anyhow::Result;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

use crate::{
  commands, hotkeys, TrayMenu, MAIN_WINDOW_NAME, OVERLAYED, SETTINGS_WINDOW_NAME,
  TRAY_OPEN_DEVTOOLS_MAIN, TRAY_OPEN_DEVTOOLS_SETTINGS, TRAY_QUIT, TRAY_RELOAD, TRAY_SETTINGS,
  TRAY_SHOW_APP, TRAY_TOGGLE_PIN,
};

pub struct Tray;

impl Tray {
  pub fn create_tray_menu(app_handle: &AppHandle) -> Result<Menu<Wry>, tauri::Error> {
    let version = app_handle.package_info().version.to_string();
    MenuBuilder::new(app_handle)
      .text(TRAY_TOGGLE_PIN, "Pin")
      .text(TRAY_SHOW_APP, "Show Overlayed")
      .text(TRAY_RELOAD, "Reload App")
      .text(TRAY_OPEN_DEVTOOLS_MAIN, "Open Devtools (main window)")
      .text(
        TRAY_OPEN_DEVTOOLS_SETTINGS,
        "Open Devtools (settings window)",
      )
      .text(TRAY_SETTINGS, "Settings")
      .separator()
      .text(OVERLAYED, format!("Overlayed v{version}"))
      .text(TRAY_QUIT, "Quit")
      .build()
  }

  pub fn update_tray(app_handle: &AppHandle) -> Result<()> {
    let menu = Tray::create_tray_menu(app_handle)?;
    let _ = TrayIconBuilder::with_id(OVERLAYED)
      .menu(&menu)
      .on_menu_event(Self::handle_menu_events)
      .build(app_handle)?;
    app_handle.manage(TrayMenu(Mutex::new(menu)));
    commands::update_tray_icon(app_handle, false);
    #[cfg(not(target_os = "macos"))]
    update_bundle_icon(app_handle, false);
    Ok(())
  }

  pub fn handle_menu_events(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
      TRAY_TOGGLE_PIN => {
        if let Err(error) = hotkeys::toggle_overlay_pin(app) {
          log::warn!("Could not toggle overlay pin from tray: {error}");
        }
      }
      TRAY_SHOW_APP => {
        if app.state::<hotkeys::OverlayVisibility>().is_hidden() {
          if let Err(error) = app.emit_to(MAIN_WINDOW_NAME, hotkeys::SHOW_OVERLAY, ()) {
            log::warn!("Could not restore overlay visibility from tray: {error}");
          }
          return;
        }
        let window = app.get_webview_window(MAIN_WINDOW_NAME).unwrap();
        window.show().unwrap();

        let placement = app.state::<crate::placement::PlacementState>();
        if placement.is_active() {
          if let Err(error) = crate::placement::reapply(&window, &placement) {
            log::warn!("Could not reapply overlay placement: {error}");
          }
        } else {
          // center and resize the window
          window.center();
          window.set_size(LogicalSize::new(400, 700)).unwrap();
        }

        window.set_focus().unwrap();
      }
      TRAY_RELOAD => {
        let window = app.get_webview_window(MAIN_WINDOW_NAME).unwrap();
        window.eval("window.location.reload();").unwrap();
      }
      TRAY_SETTINGS => {
        // find the settings window and show it
        let settings_window = app.get_webview_window(SETTINGS_WINDOW_NAME).unwrap();
        settings_window.show().unwrap();
        settings_window.set_focus().unwrap();
      }
      TRAY_OPEN_DEVTOOLS_MAIN => {
        let window = app.get_webview_window(MAIN_WINDOW_NAME).unwrap();
        window.open_devtools();
        window.show().unwrap();
      }
      TRAY_OPEN_DEVTOOLS_SETTINGS => {
        let window = app.get_webview_window(SETTINGS_WINDOW_NAME).unwrap();
        window.open_devtools();
        window.show().unwrap();
      }
      TRAY_QUIT => {
        // NOTE: we only save position and size so that StateFlags::FULLSCREEN is not saved, as
        // nspanels do not support it
        app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);
        crate::fullscreen::stop(app);
        std::process::exit(0)
      }
      _ => {}
    }
  }
}

#[cfg(not(target_os = "macos"))]
pub fn update_bundle_icon(app: &AppHandle, pinned: bool) {
  let Some(icon) = app.default_window_icon() else {
    return;
  };
  let Some(tray) = app.tray_by_id(OVERLAYED) else {
    return;
  };
  let icon = if pinned {
    match with_pin_badge(icon.rgba(), icon.width(), icon.height()) {
      Ok(rgba) => tauri::image::Image::new_owned(rgba, icon.width(), icon.height()),
      Err(error) => {
        log::warn!("Could not draw pinned tray badge: {error}");
        return;
      }
    }
  } else {
    icon.clone()
  };
  if let Err(error) = tray.set_icon(Some(icon)) {
    log::warn!("Could not apply the bundle tray icon: {error}");
  }
}

#[cfg(any(not(target_os = "macos"), test))]
fn with_pin_badge(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, &'static str> {
  let length = (width as usize)
    .checked_mul(height as usize)
    .and_then(|size| size.checked_mul(4));
  if width == 0 || height == 0 || length != Some(rgba.len()) {
    return Err("Invalid RGBA icon dimensions");
  }
  let mut result = rgba.to_vec();
  let size = f64::from(width.min(height));
  let radius = size * 0.175;
  let inset = (size * 0.03).max(0.5);
  let center_x = f64::from(width) - radius - inset;
  let center_y = f64::from(height) - radius - inset;
  let inner_radius = (radius - (size * 0.04).max(1.0)).max(0.0);
  // Four-by-four samples keep the small pinned cue smooth at tray resolutions.
  for y in (center_y - radius).floor().max(0.0) as u32..height {
    for x in (center_x - radius).floor().max(0.0) as u32..width {
      let mut white = 0;
      let mut outline = 0;
      for sy in 0..4 {
        for sx in 0..4 {
          let dx = f64::from(x) + (f64::from(sx) + 0.5) / 4.0 - center_x;
          let dy = f64::from(y) + (f64::from(sy) + 0.5) / 4.0 - center_y;
          let distance = dx * dx + dy * dy;
          if distance <= inner_radius * inner_radius {
            white += 1;
          } else if distance <= radius * radius {
            outline += 1;
          }
        }
      }
      let coverage = f64::from(white + outline) / 16.0;
      if coverage == 0.0 {
        continue;
      }
      let index = (y as usize * width as usize + x as usize) * 4;
      let background_alpha = f64::from(rgba[index + 3]) / 255.0;
      let alpha = coverage + background_alpha * (1.0 - coverage);
      let badge = (f64::from(white) * 255.0 + f64::from(outline) * 24.0) / 16.0;
      for channel in 0..3 {
        result[index + channel] = ((badge
          + f64::from(rgba[index + channel]) * background_alpha * (1.0 - coverage))
          / alpha)
          .round() as u8;
      }
      result[index + 3] = (alpha * 255.0).round() as u8;
    }
  }
  Ok(result)
}

#[cfg(test)]
mod tests {
  use super::with_pin_badge;

  #[test]
  fn badge_is_bottom_right_and_does_not_mutate_the_source() {
    let rgba = vec![80; 32 * 32 * 4];
    let original = rgba.clone();
    let result = with_pin_badge(&rgba, 32, 32).expect("valid icon");
    assert_eq!(rgba, original);
    assert_eq!(result.len(), rgba.len());
    assert_ne!(result, rgba);
    for y in 0..32 {
      for x in 0..32 {
        let index = (y * 32 + x) * 4;
        if x < 19 || y < 19 {
          assert_eq!(&result[index..index + 4], &rgba[index..index + 4]);
        }
      }
    }
    let center = (25 * 32 + 25) * 4;
    assert_eq!(&result[center..center + 4], &[255, 255, 255, 255]);
    assert!(result
      .chunks_exact(4)
      .any(|pixel| pixel == [24, 24, 24, 255]));
  }

  #[test]
  fn badge_rejects_invalid_dimensions() {
    assert!(with_pin_badge(&[], 0, 0).is_err());
    assert!(with_pin_badge(&[0; 4], 32, 32).is_err());
    assert!(with_pin_badge(&[], u32::MAX, u32::MAX).is_err());
    assert!(with_pin_badge(&[0; 4], 1, 1).is_ok());
  }

  #[test]
  fn canary_bundle_icon_uses_the_same_badge_function() {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/canary/32x32.png"))
      .expect("canary icon");
    assert_eq!((icon.width(), icon.height()), (32, 32));
    let source = icon.rgba().to_vec();
    let pinned = with_pin_badge(&source, icon.width(), icon.height()).expect("canary badge");
    assert_eq!(icon.rgba(), source);
    assert_ne!(pinned, source);
    // Optional export belongs only to this test entrypoint, never the app.
    if let Some(path) = std::env::var_os("OVERLAYED_TRAY_TEST_OUTPUT") {
      let path = std::path::PathBuf::from(path);
      std::fs::create_dir_all(&path).expect("tray test output directory");
      std::fs::write(path.join("canary-unpinned.rgba"), source).expect("source RGBA");
      std::fs::write(path.join("canary-pinned.rgba"), pinned).expect("badged RGBA");
    }
  }
}
