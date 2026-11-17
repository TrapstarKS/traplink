# Logitech keyboard controls

TrapLink can read and change Fn inversion on directly connected Logitech HID++ 2 keyboards that advertise a supported feature. This is a capability check, not a claim that every Logitech model is supported. Enabling inversion makes the F row use its secondary/media actions by default; holding Fn provides the standard function keys.

The native Devices screen reads only the selected collection. Listing devices never sends HID++ requests. Changing Fn mode requires the user to choose the control; no setting is applied at startup, from a saved profile, or during discovery.

## Supported connection and feature checks

- Vendor ID must be `046D`, with a USB or Bluetooth connection and a vendor collection `FF00:0001`, `FF00:0002`, or `FF43:0202`.
- The HID report descriptor must actually advertise compatible input/output reports: short `10` (7 bytes including ID), or long `11` (20 bytes including ID). Long reports are preferred. Ordinary keyboard collections are never written to.
- The root protocol ping must return the matching nonce and HID++ version 2 or later. Feature indices are resolved through root `getFeature`; indices are never hardcoded per model.
- Fn inversion features `40A0` and `40A2` use function 0 to read and function 1 to write. `40A3` uses the same functions with a host prefix and requires its complete four-field long reply. TrapLink resolves `1815` HOSTS_INFO, validates its active host, and addresses that exact host. It does not use the `FF` current-host shortcut because of the MX Keys S firmware issue documented by Solaar.
- Hidden or obsolete features, missing capabilities, malformed replies, permissions errors and timeouts are reported as unavailable with a reason.

USB receiver product IDs `C500`–`C5FF` are currently unsupported. Product and manufacturer names never decide compatibility. TrapLink neither guesses paired-device slots nor sends commands to slots 1–6. Connect the keyboard directly by USB or Bluetooth where the hardware offers it. A receiver appearing in inventory does not indicate its paired keyboard is configurable.

## Request and result contract

The Tauri commands `get_keyboard_settings({deviceId})` and `set_fn_inversion({deviceId, enabled})` return camelCase fields:

```ts
type KeyboardSettings = {
  deviceId: string;
  supported: boolean;
  fnInversion?: boolean;
  reason?: string;
  protocol?: string;
  featureId?: string;
};
```

`deviceId` is the same SHA-256-of-HID-path, first 12 bytes as hex, used in inventory. The frontend cannot supply an arbitrary HID path. A disappeared/invalid ID or a concurrent request produces an error. Expected read incompatibility returns `supported: false` and a reason. A set operation re-discovers the capability, reads the live value, and skips a write if it already matches. Otherwise it sends one set request, requires its matching reply, and separately reads back the requested state. Multi-host changes verify the active host before and after. A failed acknowledgement or readback returns an error explaining that a change might have applied; the UI must not present it as a confirmed success.

Every response must match device, feature, function and nonzero software ID. Notification frames and unrelated replies are ignored; Bluetooth may return device index `00` for direct `FF` requests. A reply wait is limited to 900 ms and the protocol transaction budget is 9 seconds. Unrelated input cannot extend those deadlines. Driver enumeration/open/write calls are governed by hidapi/OS timeouts, so the 9-second protocol budget is not a hard wall-clock guarantee for a stalled OS driver. Commands run off the UI thread and use a nonblocking global gate to prevent overlapping operations inside TrapLink.

No software can reserve a software ID across unrelated Logitech applications. Close other configuration tools if replies conflict. TrapLink does not persist/reapply device settings, alter pairing, switch hosts, or claim general key remapping support through this module.

## Battery level (read-only)

The device details dialog offers "Read battery" for Logitech devices. The Tauri command `get_battery_status({deviceId})` runs only on that explicit click, never during enumeration or startup, and never writes a setting. It uses the same collection/transport checks, receiver exclusion, root ping, root `getFeature` lookup, reply matching, timeouts and global gate as the Fn controls.

```ts
type BatteryStatus = {
  deviceId: string;
  supported: boolean;
  percent?: number; // 0–100
  level?: "critical" | "low" | "good" | "full";
  charging?: boolean;
  externalPower?: boolean;
  featureId?: string; // "0x1004" or "0x1000"
  reason?: string;
};
```

- `1004` UNIFIED_BATTERY is preferred. Function 0 (capabilities) must return at least 2 bytes; `percent` is reported only when the state-of-charge capability flag (`0x02`) is set. Function 1 (status) must return at least 4 bytes: state of charge (must be ≤ 100), level flags (only bits 0–3: critical/low/good/full; the highest set flag wins), charging status (0 discharging, 1 charging, 2 slow charging, 3 complete; 4 error), and external power (0 none, 1 wired, 2 wireless).
- `1000` BATTERY_STATUS is the fallback. Function 0 must return at least 3 bytes: discharge level and next level (each ≤ 100; a zero discharge level is treated as unknown), and status (0 discharging, 1 recharging, 2 almost full, 3 charged, 4 slow recharge; 5 invalid battery, 6 thermal error, 7 charging error).
- Short replies, out-of-range percentages, unknown flags or status codes, reported battery/charging errors, and replies with neither a percentage nor a level return `supported: false` with a reason. Nothing is guessed. `externalPower` for `1000` is derived from the status code only.
- Battery features behind receivers are not read, for the same reason as Fn control. Other battery features (for example `1001` voltage or `1F20` ADC measurement) are not implemented.

## Verification

Pure unit tests exercise report/argument validation, reply matching, legacy and feature errors, HID descriptor lengths, discovery, active-host validation, acknowledgement/readback, the no-write path for an unchanged value, and battery decoding for `1004`/`1000` including malformed replies, error statuses and absent/hidden features. They use scripted responses and never open hardware. Live hardware reads and user-initiated Fn changes remain separate validation steps; successful unit tests do not certify a particular keyboard model or firmware.

## Protocol sources

- [Logitech HID++ 2.0 packet and feature documentation](https://github.com/Logitech/cpg-docs/blob/master/hidpp20/README.rst)
- [Logitech IRoot: getFeature and protocol ping](https://github.com/Logitech/cpg-docs/blob/master/hidpp20/features/0x0000-IRoot.rst)
- [Solaar FnSwap, NewFnSwap and K375sFnSwap protocol usage](https://github.com/pwr-Solaar/Solaar/blob/master/lib/logitech_receiver/settings_templates.py)
- [Solaar HOSTS_INFO and new Fn inversion reads](https://github.com/pwr-Solaar/Solaar/blob/master/lib/logitech_receiver/hidpp20.py)
- [Solaar transport, Bluetooth reply matching and errors](https://github.com/pwr-Solaar/Solaar/blob/master/lib/logitech_receiver/base.py)
- [HID collection/report examples from hardware measurements](https://github.com/abd3lraouf-studios/logitech-layout-auto-switcher/blob/main/docs/PROTOCOL.md)
- [Solaar UNIFIED_BATTERY and BATTERY_STATUS decoding](https://github.com/pwr-Solaar/Solaar/blob/master/lib/logitech_receiver/hidpp20.py)
- [libratbag HID++ 2.0 battery features](https://github.com/libratbag/libratbag/blob/master/src/hidpp20.c)
- [Multi-host Fn reply length verification](https://github.com/AprilNEA/OpenLogi/pull/1782)

The implementation is independent Rust code. These sources establish the wire format and capability semantics; no GPL implementation code is copied into TrapLink.
