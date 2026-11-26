export type DeviceErrorLike = { error?: unknown; code?: unknown; status?: unknown } | null;

// The server deliberately reports unknown and expired codes the same way (no existence oracle),
// so the copy must cover both: a typo is fixable, an expired code needs a new one.
export function deviceError(error: DeviceErrorLike, fallback: string) {
  const code = error?.error ?? error?.code;
  if (error?.status === 429) return "Too many attempts. Wait before checking another code.";
  if (error?.status === 401) return "Your session expired. Sign in again to authorize the app.";
  if (code === "access_denied" || error?.status === 403) {
    return "This code was started in another browser session or account. Continue there, or generate a new code in the app.";
  }
  if (code === "expired_token") return "This code is invalid or has expired. Check each character against the app, or generate a new code.";
  if (typeof error?.status === "number" && (error.status === 0 || error.status >= 500)) return "The service is unavailable. Try again shortly.";
  return fallback;
}
