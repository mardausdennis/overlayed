import Config, { type HotkeyBinding, type HotkeyCommand, type OverlayedConfig } from "@/config";
import { useAppStore } from "@/store";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect } from "react";
import { flushSync } from "react-dom";
import type { PlacementMonitors } from "./use-placement";

export interface HotkeyResult extends HotkeyBinding {
  ok: boolean;
  error: string | null;
}

export interface HotkeysStatus {
  bindings: HotkeyResult[];
  supported: boolean;
  suspended: boolean;
}

interface VisibilityStatus {
  hotkeyHidden: boolean;
  fullscreenHidden: boolean;
  visible: boolean;
  revision: number;
}

const mainWindow = getCurrentWebviewWindow().label === "main";

export const useHotkeys = () => {
  useEffect(() => {
    if (!mainWindow) return;
    let disposed = false;
    let config: OverlayedConfig | null = null;
    let configSeen = false;
    let bindingsKey: string | null = null;
    let registrations = Promise.resolve();
    let actions = Promise.resolve();
    let tracking = Promise.resolve();
    let trackingEnabled: boolean | null = null;
    let visibility: VisibilityStatus | null = null;

    const receiveConfig = (value: OverlayedConfig) => {
      config = value;
      if (trackingEnabled !== value.hideWhenFullscreen) {
        trackingEnabled = value.hideWhenFullscreen;
        tracking = tracking
          .then(async () => {
            if (!disposed) await invoke("set_fullscreen_tracking", { enabled: value.hideWhenFullscreen });
          })
          .catch(error => {
            trackingEnabled = null;
            console.warn("Could not update fullscreen tracking", error);
          });
      }
      const key = JSON.stringify(value.hotkeys);
      if (key === bindingsKey) return;
      bindingsKey = key;
      registrations = registrations
        .then(async () => {
          if (!disposed) await invoke<HotkeysStatus>("set_hotkeys", { bindings: value.hotkeys });
        })
        .catch(error => {
          bindingsKey = null;
          console.warn("Could not update global hotkeys", error);
        });
    };

    const receiveVisibility = (value: VisibilityStatus) => {
      if (!visibility || value.revision >= visibility.revision) {
        visibility = value;
        // A native hide can arrive while a previous show waits for two frames.
        // Apply it immediately rather than leaving opacity visible in the queue.
        if (!value.visible) useAppStore.getState().setAppVisible(false);
      }
    };

    const reconcileVisibility = async () => {
      if (!visibility) return;
      const status = visibility;
      if (!status.visible) {
        // Native transitions are published only after enabling click-through.
        useAppStore.getState().setAppVisible(false);
      } else {
        // Paint first, then restore cursor handling from the current native pin.
        // No show(), hide() or focus call: the foreground app stays active.
        flushSync(() => useAppStore.getState().setAppVisible(true));
        await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
        if (disposed) return;
        receiveVisibility(await invoke<VisibilityStatus>("acknowledge_overlay_visible", { revision: status.revision }));
        if (!visibility?.visible) useAppStore.getState().setAppVisible(false);
      }
    };

    const setVisible = async (visible: boolean) => {
      receiveVisibility(await invoke<VisibilityStatus>("set_overlay_visible", { visible }));
      await reconcileVisibility();
    };

    const nextMonitor = async () => {
      if (config?.placementMode !== "anchored") return;
      const response = await invoke<PlacementMonitors>("get_monitors");
      if (!response.positioningSupported || config?.placementMode !== "anchored") return;
      const monitors = [...response.monitors].sort((a, b) => a.x - b.x || a.y - b.y);
      if (monitors.length < 2) return;
      const current =
        monitors.find(monitor => monitor.name === config?.placementMonitor) ??
        monitors.find(monitor => monitor.primary) ??
        monitors[0];
      const next = monitors[(monitors.indexOf(current!) + 1) % monitors.length];
      if (!next) return;
      await Config.set("placementMonitor", next.name || null);
      const value = await Config.getConfig();
      receiveConfig(value);
      await emit("config_update", value);
    };

    const enqueue = (action: () => Promise<void>) => {
      actions = actions
        .then(async () => {
          if (!disposed) await action();
        })
        .catch(error => console.warn("Could not handle overlay hotkey", error));
    };

    const listeners = Promise.all([
      listen<OverlayedConfig>("config_update", event => {
        configSeen = true;
        receiveConfig(event.payload);
      }),
      listen<HotkeyCommand>("hotkey_action", event => {
        if (event.payload === "toggle-visibility") {
          enqueue(async () => {
            if (!visibility) receiveVisibility(await invoke<VisibilityStatus>("get_overlay_visibility"));
            // Toggle the user's reason, not effective visibility. Fullscreen
            // leaving must never undo a separate hide-hotkey choice.
            await setVisible(visibility?.hotkeyHidden ?? false);
          });
        }
        if (event.payload === "next-monitor") enqueue(nextMonitor);
      }),
      listen("show_overlay", () => enqueue(() => setVisible(true))),
      listen<VisibilityStatus>("overlay_visibility", event => {
        receiveVisibility(event.payload);
        enqueue(reconcileVisibility);
      }),
    ]);
    listeners
      .then(async unlisteners => {
        if (disposed) {
          unlisteners.forEach(unlisten => unlisten());
          return;
        }
        const value = await Config.getConfig();
        if (!disposed && !configSeen) receiveConfig(value);
        // Read after config push and after listening, closing the startup/reload
        // gap without resetting either native hide reason.
        await tracking;
        if (!disposed) {
          receiveVisibility(await invoke<VisibilityStatus>("get_overlay_visibility"));
          enqueue(reconcileVisibility);
        }
      })
      .catch(error => console.warn("Could not initialize overlay hotkeys", error));

    return () => {
      disposed = true;
      listeners
        .then(unlisteners => unlisteners.forEach(unlisten => unlisten()))
        .catch(error => console.warn("Could not remove overlay hotkey listeners", error));
    };
  }, []);
};
