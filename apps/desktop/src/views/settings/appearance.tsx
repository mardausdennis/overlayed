import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import Config, { type OverlayedConfig, type RowStyle } from "@/config";
import { useConfigValue } from "@/hooks/use-config-value";
import { emit } from "@tauri-apps/api/event";
import { useState } from "react";

export const Appearance = () => {
  const { value: rowStyle } = useConfigValue("rowStyle");
  const { value: namesOnlyWhileSpeaking } = useConfigValue("namesOnlyWhileSpeaking");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const update = async <K extends "rowStyle" | "namesOnlyWhileSpeaking">(key: K, value: OverlayedConfig[K]) => {
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
        <span className="font-medium">Row style</span>
        <div className="inline-flex rounded-md border border-zinc-700" role="group" aria-label="Row style">
          {(["classic", "pill"] as RowStyle[]).map(style => (
            <Button
              key={style}
              type="button"
              variant={rowStyle === style ? "secondary" : "ghost"}
              size="sm"
              className="h-7 rounded-none first:rounded-l-md last:rounded-r-md"
              aria-pressed={rowStyle === style}
              disabled={saving}
              onClick={() => update("rowStyle", style)}
            >
              {style === "classic" ? "Classic" : "Pill"}
            </Button>
          ))}
        </div>
      </div>
      <div className="flex items-center justify-between h-8 mx-2">
        <label htmlFor="names-only-while-speaking" className="font-medium">
          Show names only while speaking
        </label>
        <Switch
          id="names-only-while-speaking"
          checked={namesOnlyWhileSpeaking}
          disabled={saving}
          onCheckedChange={checked => update("namesOnlyWhileSpeaking", checked)}
        />
      </div>
      {error && (
        <p className="mx-2 text-xs text-red-400" role="alert">
          {error}
        </p>
      )}
    </div>
  );
};
