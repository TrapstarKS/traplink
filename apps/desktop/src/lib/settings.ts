import type { Settings } from "@traplink/shared";

// Keep this file free of Tauri imports: it is unit tested under Node.

/**
 * Returns the latest settings with only the fields the user edited (where the
 * draft differs from the baseline it started from) applied on top, so a stale
 * draft cannot undo background changes such as an emergency stop.
 * Remapping is owned by the Profiles page and always keeps the latest value.
 */
export function mergeSettingsDraft(latest: Settings, baseline: Settings, draft: Settings): Settings {
  const merged: Record<string, unknown> = { ...latest };
  for (const key of Object.keys(draft) as (keyof Settings)[]) {
    if (key !== "remapEnabled" && draft[key] !== baseline[key]) merged[key] = draft[key];
  }
  return merged as unknown as Settings;
}
