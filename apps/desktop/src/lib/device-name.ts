// The rule is shared with the web tests through packages/shared. It matches the server rule
// in apps/web/src/server/devices.ts and the Rust `model::validate_device_name`.
export { deviceNameValid, maxDeviceNameLength } from "@traplink/shared";
