// Keep this file free of Tauri imports: it is unit tested under Node.

/**
 * Returns a runner that ignores new calls while an earlier task is still running,
 * so a double click cannot start the same request twice during an async check.
 * Returns undefined for an ignored call.
 */
export function singleFlight(): <T>(task: () => Promise<T>) => Promise<T | undefined> {
  let busy = false;
  return async task => {
    if (busy) return undefined;
    busy = true;
    try { return await task(); } finally { busy = false; }
  };
}
