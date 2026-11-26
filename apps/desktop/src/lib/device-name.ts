// Shared with the server rule in apps/web/src/server/devices.ts and the Rust
// `model::validate_device_name`: trim, then 1 to 80 Unicode code points and no
// control characters. Keep this file free of Tauri imports so it can be tested.
export const maxDeviceNameLength = 80;

export function deviceNameValid(value: string): boolean {
  const name = value.trim();
  const length = Array.from(name).length;
  return length >= 1 && length <= maxDeviceNameLength && !/\p{Cc}/u.test(name);
}
