import { useSyncExternalStore } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { nativeAvailable } from "@/lib/bridge";

export interface AvailableUpdate { version: string; date?: string; notes?: string }

export type UpdaterState =
  | { status: "unsupported" }
  | { status: "idle" }
  | { status: "checking" }
  | { status: "current"; checkedAt: number }
  | { status: "available"; update: AvailableUpdate }
  | { status: "downloading"; update: AvailableUpdate; downloaded: number; total?: number }
  | { status: "installing"; update: AvailableUpdate }
  | { status: "installed"; update: AvailableUpdate }
  | { status: "error"; message: string; update?: AvailableUpdate };

export interface UpdaterSnapshot { state: UpdaterState; currentVersion: string | null; focusRequest: number }

let snapshot: UpdaterSnapshot = { state: { status: nativeAvailable ? "idle" : "unsupported" }, currentVersion: null, focusRequest: 0 };
let pending: Update | null = null;
let startupChecked = false;
const listeners = new Set<() => void>();

function set(next: Partial<UpdaterSnapshot>) {
  snapshot = { ...snapshot, ...next };
  listeners.forEach(listener => listener());
}
function subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; }
function describe(update: Update): AvailableUpdate { return { version: update.version, date: update.date, notes: update.body?.trim() || undefined }; }
function busy(state: UpdaterState) { return state.status === "checking" || state.status === "downloading" || state.status === "installing"; }

if (nativeAvailable) {
  getVersion().then(version => set({ currentVersion: version }), () => undefined);
}

export function updaterMessage(error: unknown): string {
  const text = typeof error === "string" ? error : error instanceof Error ? error.message : "";
  if (/signature|minisign|public key|pubkey/i.test(text)) return "The update signature couldn't be verified. Nothing was installed.";
  if (/valid release JSON|404|not found|status code/i.test(text)) return "Couldn't get information about the latest version. Try again later.";
  if (/error sending request|dns|connect|timed out|timeout|network|offline/i.test(text)) return "Couldn't reach the update server. Check your connection and try again.";
  if (/permission|access is denied|elevat|administrator/i.test(text)) return "The system didn't allow the update to install. Check permissions and try again.";
  return "Couldn't complete the update. Try again.";
}

export async function checkForUpdates(): Promise<AvailableUpdate | null> {
  if (!nativeAvailable || busy(snapshot.state)) return null;
  set({ state: { status: "checking" } });
  try {
    const update = await check();
    const previous = pending;
    pending = update;
    if (previous && previous !== update) void previous.close().catch(() => undefined);
    if (!update) { set({ state: { status: "current", checkedAt: Date.now() } }); return null; }
    const info = describe(update);
    set({ state: { status: "available", update: info } });
    return info;
  } catch (error) {
    set({ state: { status: "error", message: updaterMessage(error) } });
    return null;
  }
}

export async function checkOnStartup(): Promise<AvailableUpdate | null> {
  if (!nativeAvailable || startupChecked) return null;
  startupChecked = true;
  return checkForUpdates();
}

export async function installUpdate(): Promise<void> {
  const update = pending;
  if (!update || busy(snapshot.state)) return;
  const info = describe(update);
  let downloaded = 0;
  let total: number | undefined;
  set({ state: { status: "downloading", update: info, downloaded, total } });
  try {
    await update.downloadAndInstall(event => {
      if (event.event === "Started") { total = event.data.contentLength || undefined; set({ state: { status: "downloading", update: info, downloaded, total } }); }
      else if (event.event === "Progress") { downloaded += event.data.chunkLength; set({ state: { status: "downloading", update: info, downloaded, total } }); }
      else if (event.event === "Finished") set({ state: { status: "installing", update: info } });
    });
    set({ state: { status: "installed", update: info } });
  } catch (error) {
    set({ state: { status: "error", message: updaterMessage(error), update: info } });
    return;
  }
  try { await relaunch(); }
  catch { set({ state: { status: "error", message: "The update was installed, but TrapLink didn't restart. Close and reopen the app to finish.", update: info } }); }
}

export function requestUpdatesFocus() { set({ focusRequest: snapshot.focusRequest + 1 }); }

export function useUpdater(): UpdaterSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot, () => snapshot);
}
