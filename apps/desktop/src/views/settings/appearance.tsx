import { Switch } from "@/components/ui/switch";
import Config, { type OverlayedConfig } from "@/config";
import { useConfigValue } from "@/hooks/use-config-value";
import { emit } from "@tauri-apps/api/event";
import { platform } from "@tauri-apps/plugin-os";
import { useState } from "react";

export const Appearance = () => {
  const { value: showChannelName } = useConfigValue("showChannelName");
  const { value: hideWhenFullscreen } = useConfigValue("hideWhenFullscreen");
  const windows = platform() === "windows";
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const update = async <K extends "showChannelName" | "hideWhenFullscreen">(key: K, value: OverlayedConfig[K]) => {
    setSaving(true);
    try {
      await Config.set(key, value);
      await emit("config_update", await Config.getConfig());
      setError(null);
    } catch (error) {
      console.warn("Could not save overlay appearance", error);
      setError("Couldn't save this setting. Try again.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="mt-4 border-t border-zinc-700 pt-3 flex flex-col gap-2 text-sm" data-appearance-panel>
      <div className="flex items-center justify-between h-8 mx-2">
        <label htmlFor="show-channel-name" className="font-medium">
          Show channel name
        </label>
        <Switch
          id="show-channel-name"
          checked={showChannelName}
          disabled={saving}
          onCheckedChange={checked => update("showChannelName", checked)}
        />
      </div>
      <div className="flex items-center justify-between h-8 mx-2">
        <label htmlFor="hide-when-fullscreen" className="font-medium">
          Hide when a fullscreen app is in front
        </label>
        <Switch
          id="hide-when-fullscreen"
          checked={hideWhenFullscreen}
          disabled={saving || !windows}
          onCheckedChange={checked => update("hideWhenFullscreen", checked)}
        />
      </div>
      {!windows && <p className="mx-2 text-xs text-zinc-400">Windows only for now.</p>}
      {error && (
        <p className="mx-2 text-xs text-red-400" role="alert">
          {error}
        </p>
      )}
    </div>
  );
};
