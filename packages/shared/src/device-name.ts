// The desktop device name rule. It must match the server rule in
// apps/web/src/server/devices.ts and the Rust `model::validate_device_name`: trim, then
// 1 to 80 Unicode code points and no control characters (Unicode category Cc).
// This lives in packages/shared so the web tests can check it against the server rule
// without importing apps/desktop, which the web Docker build does not copy.
export const maxDeviceNameLength = 80;

export function deviceNameValid(value: string): boolean {
  const name = value.trim();
  const length = Array.from(name).length;
  return length >= 1 && length <= maxDeviceNameLength && !/\p{Cc}/u.test(name);
}
