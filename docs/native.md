# Native desktop implementation

TrapLink's native app is Tauri 2/Rust with a React interface. Windows and macOS are targets. Local use does not require an account. The browser preview cannot enumerate devices, connect peers, write settings or claim a transfer succeeded.

## Run and build

From the repository root, install JavaScript dependencies with `pnpm install`. Run `pnpm --filter @traplink/desktop tauri dev`. The Rust toolchain and platform Tauri dependencies are required. On Windows, install Visual Studio C++ Build Tools and WebView2. On macOS, install Xcode command-line tools. A release build uses `pnpm --filter @traplink/desktop tauri build`; build each operating system on its own runner. The Windows bundle targets NSIS and the macOS bundle targets DMG. Release macOS builds are signed with the stable self-signed TrapLink certificate (secrets `MAC_CERT_P12`/`MAC_CERT_PASSWORD`, kept outside the repository; see `scripts/runner/README.md`). They are not notarized. A local macOS build without that certificate must opt into ad-hoc signing with `--config '{"bundle":{"macOS":{"signingIdentity":"-"}}}'`.

Native checks, from `apps/desktop/src-tauri`:

```powershell
$env:CARGO_BUILD_JOBS = '2'
cargo check --locked
cargo test --locked
cargo fmt --all -- --check
```

## Data and credentials

The platform app data folder for `app.traplink.desktop` contains a versioned `settings.json` and `backups/<UUID>.json`. Writes use a temporary file, sync and atomic replacement. Profiles are editable records; mappings are applied in software by Remapping on Windows and are **never written to Logitech hardware**. Backup import checks the format/version, sizes and profile identifiers. A backup contains settings and profiles, but no device identity, peer trust, account token or clipboard data. Restore imports profiles, name and theme, preserves this computer's identity/server/download location, and switches sharing off. Exported backups are plain JSON; users should choose their storage location accordingly.

Static X25519 identity private keys and online device credentials use Windows Credential Manager or macOS Keychain through `keyring`. Credentials are never written into the JSON files. If the credential store fails, the interface reports the failure and LAN pairing is unavailable. Input sharing is off by default. The app restores opted-in input settings and saved Flow edges at launch after registering the emergency shortcut.

## LAN protocol and pairing

The native process listens on TCP port 48372 (configurable). A firewall prompt may appear. Do not expose this port through a router. Create an invitation on one computer, copy the complete code, and paste it into **Use an invitation** on the other computer. Address and secret travel together; no separate IP or port entry is required. For a trusted peer, use Connect. There is no discovery broadcast and no relay. Automatic reconnect exists only for peers with Flow enabled (see "Connection resilience" under Flow). It only dials already-trusted peers with pinned keys and never pairs.

Every session socket sets `TCP_NODELAY` and TCP keepalive (first probe after 5 s, then every 1 s). Each frame goes out in one write with a 3 s write timeout. Peers on protocol revision 2 announce it in the encrypted hello. They exchange an encrypted `probe`/`probe_reply` every second, which measures round-trip time (smoothed) and proves the link is alive. Older peers get the previous 15 s ping instead. A revision-2 session is declared dead after 3 s of silence while input is active, or after 20 s otherwise. Older peers keep the previous 60 s. An intentional Disconnect sends an encrypted `goodbye` to revision-2 peers, so the other side knows not to reconnect.

Creating an invitation is consent to accept one peer using a random 256-bit secret. The `traplink:v1:` code contains base64url JSON with the secret, receiver public key, expiration, and up to eight numeric private IPv4 addresses. It excludes public, loopback and link-local interfaces when generated; common 192.168 and 172.16/12 networks are tried before 10/8, including VPN addresses. This ordering is a heuristic, not physical-interface detection. The parser rejects unknown versions or fields, malformed keys, duplicates, public addresses, hostnames, invalid ports, expired codes, expirations more than five minutes away, and codes over 2 KiB. It never resolves hosts or opens the code as a URL. The underlying address validator retains local/loopback support for existing connections and transport tests.

The code expires after five minutes and is consumed on authenticated use. Candidates are tried sequentially within a total 12-second budget; a failed TCP connection or early handshake can fall through to the next address. The receiver identity is verified before the final handshake message. After that message may have been sent, a failure is returned without retrying a potentially consumed invitation. The initiating and receiving computers use `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s` through the `snow` implementation. The initial shared secret is deliberately long, not a short PIN vulnerable to guessing. Subsequent sessions use Noise XX with both static keys pinned to the saved trust record. Device names/IDs are sent only after encryption; peer key changes are rejected. Fingerprints are SHA-256 of each static public key. Eight concurrent handshakes, bounded frames and a 12-second receiver handshake timeout limit unauthenticated resource use.

Both global sharing switches and local per-peer grants are off by default. Pairing itself grants no clipboard, file or input access. Clipboard transfers are manual text-only sends, limited to 24 KiB. Files are selected explicitly, streamed in encrypted chunks, limited to 128 MiB and saved under the chosen downloads folder's `TrapLink` directory with a new UUID prefix. The receiver rejects unsafe names, does not execute files, never overwrites an existing file, and reports completion only after validating the byte count and syncing the received file. Failed/incomplete temporary files are removed. Transfer history is in memory only.

## Focused input control

The first input prototype controls the remote computer from an explicitly focused control area in the local app. It transmits common keyboard codes, normalized mouse position, left/middle/right buttons and wheel events over the authenticated session. The receiver uses `enigo` to inject input on Windows/macOS. The receiver's primary display is the pointer coordinate target. There is no screen video stream. Screen-edge switching is described under Flow below. Keyboard layout differences and platform-reserved shortcuts can behave differently; secure desktops and privilege boundaries remain controlled by the OS.

Open Connections, choose Manage on the paired computer and start manual control. The receiver must grant access and acknowledge the session before the local control pane sends input. Escape, loss of focus and closing the pane end focused control. Emergency stop releases tracked keys and buttons and disables input sharing and button remapping. The global shortcut is Ctrl+Alt+Esc on Windows and Control+Option+Esc on macOS. A three-second inactivity watchdog releases held input after the peer stops responding.

On macOS, allow TrapLink under **System Settings → Privacy & Security → Accessibility** before receiving control. Focused control needs no Input Monitoring permission because source input is confined to the focused app pane; sending Flow from a Mac needs both permissions (see Flow). Permission status is not queried in this build; the UI reports it as unverified, and native injection errors describe missing access. Real Windows-to-macOS testing remains necessary before calling this production-ready universal Flow.

## Flow (screen-edge switching)

Flow lets the pointer cross a chosen outer screen edge to a paired computer. Open **Connections → Manage → Flow** to choose the edge and switch method. Flow starts off. TrapLink saves a setup once enabled and can restore it at launch when access is available. Each handoff uses the authenticated connection and requires acknowledgement from the receiver.

- **Capture platform.** Windows uses `WH_MOUSE_LL`/`WH_KEYBOARD_LL` hooks on a dedicated thread with its own message loop. macOS uses one active Quartz event tap (`CGEventTapCreate` at the HID location, falling back to the session location, inserted at the head) on a dedicated thread with its own `CFRunLoop`; it listens to mouse moves and drags, buttons, the scroll wheel, key down/up and modifier changes. Only public Apple APIs are used. Local input is swallowed only while forwarding; otherwise every event passes through unchanged. Injected events (Windows' injected flag; on macOS events posted by this process or stamped by `enigo`) are ignored, which prevents feedback loops.
- **macOS permissions.** Arming Flow (and only then) checks Accessibility (`AXIsProcessTrusted`) and Input Monitoring (`CGPreflightListenEventAccess`). Missing permissions are requested through the system prompts (`AXIsProcessTrustedWithOptions` with the prompt option, `CGRequestListenEventAccess`), and arming fails with a message asking the user to enable TrapLink under **System Settings → Privacy & Security → Accessibility** and **Input Monitoring**, then quit and reopen the app. If the tap cannot be created, arming fails too; success is never reported without a running tap. macOS ties these permissions to the app's code signature. Releases are signed with one stable self-signed certificate, so grants persist across updates; the first self-signed install after an ad-hoc signed version (0.1.8 or earlier) needs the permissions granted once more, and a locally built ad-hoc app may need to be allowed again.
- **macOS tap health.** macOS disables a tap whose callback is too slow (`kCGEventTapDisabledByTimeout`) or because of user input (`kCGEventTapDisabledByUserInput`). The tap is re-enabled immediately and the event is reported like a removed Windows hook (`hook_removed`), returning control locally. Every 500 ms the run loop also refreshes the display layout and re-enables a tap found disabled; Repair connection recreates the tap.
- **Protocol.**
  1. The sender sends a Flow start with its edge and normalized entry position. Protocol 4 also assigns a unique session ID to this visit.
  2. The receiver validates the request, places its pointer just inside the opposite outer edge of its monitor layout, and acks.
  3. The controller drains relative motion on a 4 ms timer and flushes pending motion before keys and buttons. The sender bounds each motion packet to ±4096 per axis. Protocol 4 carries the visit ID on input, keepalives, returns and stops. Older peers use the existing ordered messages.
  4. Pushing past the outer return edge releases the receiver’s held keys and buttons. On protocol 4, the sender restores its cursor when the matching return arrives and replies with a scoped stop. Late packets from an earlier visit cannot take control of the newer one.
- **Pointer movement.** The sender parks its cursor and sends relative deltas. The receiver integrates a burst from its last injected position and places the pointer absolutely. After an idle interval it samples the OS cursor again. Windows uses virtual-desktop pixels and macOS uses Quartz points.
- **Monitor geometry (Windows).** The hook thread and the receiver run per-monitor DPI aware (V2), so all coordinates are physical pixels of the virtual desktop. Those can be negative for monitors left of or above the primary one. The monitor list is re-read when the virtual-desktop metrics change and at least every 5 s. Only *outer* edges count: the side of a monitor where no other monitor continues at that position. A seam between two monitors never switches, and in a non-rectangular layout the exposed part of a shorter monitor's edge does. The Windows receiver moves within its whole virtual desktop through `SendInput` absolute virtual-desk coordinates.
- **Monitor geometry (macOS).** Quartz provides the position and bounds of each active display in global points. TrapLink uses those coordinates for moves, drags, clicks and releases on Retina and external displays. While forwarding, the event tap captures physical deltas, including accumulated fractional motion. On return it restores the cursor’s association with the mouse and places the pointer at the matching edge.
- **Keys.** Keys travel as DOM `code` names for physical keys (letters, digits, punctuation, numpad, F1–F12, navigation, arrows, Caps Lock, IntlBackslash, IntlRo, NumpadComma and left/right modifiers). Each receiver injects the physical key (Windows: the scan code's virtual key in the foreground layout, numpad keys explicitly; macOS: the virtual key code, honouring the ISO swap of the two keys around Shift and "1"), so ABNT2/ISO/US punctuation comes from the receiver's own layout. Both computers should use the same physical layout. Modifiers on the wire are PC-logical: a Mac sender forwards **Command as Control** and **Control as the Windows key**, so Cmd+C, Cmd+V, Cmd+Z and similar shortcuts become Ctrl shortcuts on Windows; Option is Alt. A Mac receiver maps them back, so Mac-to-Mac keeps Command. Ctrl mode and the emergency use the physical Control key on a Mac: **Control+Option+Esc** returns control. Keys without a PC equivalent (Fn, media keys) are swallowed while forwarding and never sent. A receiver ignores keys it cannot type instead of ending the session.
- **Switch modes.**
  - **Edge only** uses the chosen delay, from 0 to 1000 ms, with 250 ms as the default. With a delay, the pointer must keep pushing into the edge. A single outward push is enough with a zero delay.
  - "Hold Ctrl + edge" switches on the first push, but only while Ctrl is held.
  - A successful return re-arms the edge for a deliberate outward push from the inset. After a rejected start, move at least 24 px back into the screen before trying again.
  - Pulling the pointer back into the screen or moving onto another display while the peer is confirming a switch keeps control local. A subsequent push starts a fresh switch.
  - "Block switching while a mouse button is held" (on by default) blocks a switch while any mouse button is held.
  - An edge can be linked to only one peer. The 0–1 entry position is measured along the whole virtual desktop.
- **Safety.**
  - `Ctrl+Alt+Escape` is detected inside the keyboard hook, because swallowed modifiers would hide the global shortcut. It always returns control locally, releases keys, and runs the existing emergency stop.
  - The sender tracks which computer owns each held key and button. The receiver releases its held inputs when the visit ends. **Return control** releases the current visit while preserving the Flow setup.
  - The receiver's three-second watchdog still applies, and the sender sends a keepalive every second.
  - If the controller task stalls for more than 2 s, the hook fails open and stops swallowing input.
  - Disconnecting, revoking a permission, turning Flow off, a lost connection, or a failed or timed-out start (3 s) returns control. A failed start leaves the edge disarmed until the pointer leaves it.
  - The switch is a small state machine (`Local`, `Starting`, `Forwarding`). Every exit leads back to `Local`: return, loss of the peer, emergency, hook loss, and the start deadline (3.5 s). A 250 ms health check also releases control if forwarding and the session state ever disagree. A randomized table test exercises this.
- **Hook health (Windows).** The hook callbacks only update atomics and a small shared state; they never block on the network. Every 500 ms the hook thread compares real cursor movement with the events its hook saw. If the cursor moved during two consecutive checks while the hook saw nothing, Windows has silently removed it (this happens after a callback exceeds the OS timeout). Both hooks are then reinstalled, at most once every 5 s. If that happens while forwarding, control returns locally and the UI shows "Capture reinstalled" with the reason. A failed reinstall is reported as "Capture unavailable". "Repair connection" also reinstalls the hooks.
- **Connection resilience.** Only applies to peers with Flow enabled, which are already paired, trusted and key-pinned. Reconnecting uses the same pinned Noise XX handshake as Connect, so authentication is never weakened and nothing is paired automatically.
  - When the session drops, control returns locally at once and the peer's edge is suspended.
  - The app then redials the last-good address with exponential backoff: 500 ms base, doubling, capped at 15 s, with ±20 % jitter, for at most 10 attempts.
  - A resume from sleep (a wall-clock jump of more than 5 s) or a change in local IPv4 addresses restarts the attempts immediately.
  - To avoid both sides dialling at once, the computer with the larger device ID waits an extra 1.5 s.
  - When the peer intentionally disconnects (`goodbye`), this side does not reconnect.
  - After the attempts run out, the UI shows "No connection".
  - "Repair connection" ends any control and drops the session. It reinstalls the hooks, resets the attempt counter and dials once immediately. Only a connection the native side actually re-established is reported as success.
  - Once the session is back, the edge is re-armed but requires the pointer to leave the edge first.
- **Status in the interface.** The peer's Flow section shows the observed state:
  - Ready
  - Controlling
  - Switching…
  - Reconnecting… (attempt N of M)
  - Reconnecting in N s
  - No connection
  - Other computer disconnected
  - Disconnected by the other computer
  - Capture reinstalled / stopped

  The panel shows link round-trip time for compatible peers. After a successful switch, it shows the time from the edge trigger to local capture, excluding the configured dwell. After a confirmed return, it shows the time from crossing the remote return edge to release confirmation. The panel also explains the most recent error.
  Reported codes include `connection_lost`, `reconnect_failed`, `remote_closed`, `hook_removed`, `hook_failed`, `permission_changed`, `peer_denied`, `peer_busy`, `peer_accessibility`, `timeout` and `start_failed`.
- **Copy and paste ("Copy and paste between computers").** This is an optional per-peer setting, off by default. It requires the global clipboard switch and the per-peer clipboard grant. When control switches to the peer, the sender sends its current clipboard text once, through the existing manual text path and its 24 KiB limit. When control returns, the receiver sends its clipboard text back under the same rules, checked on the receiver's side. Clipboard contents are never logged.

Limitations:
- Flow, reconnect, hook reinstall and edge pressure have not been tested between two real machines. The tests are pure logic and local TCP on one machine. Real Wi-Fi roaming, sleep/resume, firewalls, mixed-DPI monitor sets and the actual Windows hook-timeout behaviour still need manual verification.
- Extra mouse buttons, Fn and media keys are not forwarded. Caps Lock is forwarded as a press when its lock toggles; the local Caps Lock state of a Mac sender can still toggle. On macOS, secure text fields (Secure Event Input) hide keystrokes from the tap, and trackpad gestures other than scrolling are not captured.
- Movement deltas are not scaled for DPI differences between the two computers.
- macOS capture has been built and unit-tested in CI only; it has not been exercised with real devices. Pointer movement is in points on a Mac and in physical pixels on Windows, so speed differs between Retina and standard displays.
- Silent hook removal is detected heuristically (the cursor moved but no events arrived). Removal while the mouse is idle is detected on the next movement. UAC and secure-desktop prompts remain outside the hooks' reach.
- Probes, `goodbye`, 3 s half-open detection and latency need both computers on protocol revision 2. With an older peer, the 60 s timeout and 15 s ping still apply. TCP keepalive is the only faster backstop, and no latency is shown.
- Reconnect retries only the last-good address (or the saved address). It does not discover a new IP after the peer moves to another network; reconnect manually or create a new invitation.
- Older TrapLink builds reject Flow wire messages.

## Remapping (per-application, Windows)

- Off by default (`settings.remapEnabled`, absent in older files means off). It is toggled only from the Profiles page with `set_remap_enabled`. Ctrl+Alt+Esc, the emergency button and restoring a backup turn it off and save it as off. It is never remapped itself.
- Profiles keep the `{from, to}` string mappings, so no migration is needed. `from` is one DOM `KeyboardEvent.code` (aliases like `F1` or `a` are accepted) or `MouseMiddle`/`MouseBack`/`MouseForward`. `to` is a key (`KeyB`), a shortcut (`Ctrl+Shift+KeyC`, where modifiers press first and release in reverse) or `disabled`. Strings that do not parse are kept and ignored, and the status reports them per profile.
- Profiles with no application apply everywhere. A profile with an application overlays them when the foreground process's executable file name matches it, compared case-insensitively with folders and `.exe` ignored. Roblox's `…\Roblox\Versions\version-*\RobloxPlayerBeta.exe` keeps matching across updates.
- Profiles limited to one device are saved but not applied, because low-level hooks cannot tell which keyboard sent a key. TrapLink's own window is never remapped, so key capture sees physical keys.
- `src-tauri/src/remap.rs` compiles profiles into fixed 515-slot tables (scan code | extended << 8, plus 3 mouse buttons).
- `flow.rs` owns a single WH_KEYBOARD_LL/WH_MOUSE_LL thread, which is reference-counted by Flow and remapping (`flow::hooks::acquire/release`). Each physical event goes to Flow's capture logic, then to `remap::hook_key/hook_mouse`. Remapping does one array lookup behind a bounded `try_lock` and falls back to pass-through. Output is sent with `SendInput` (scan codes, or a virtual key for media keys and NumLock/PrintScreen) after the lock is dropped.
- Injected input, including TrapLink's own, is never remapped.
- Foreground changes come from `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` on the hook thread, with a 500 ms timer as fallback. A pure state machine (`Engine`) pairs every down with its up. Anything held is released on a profile switch, master off, emergency or window close. An orphaned physical key-up is swallowed. Auto-repeat repeats only the output key.
- While Flow forwards to another computer, remapping steps aside and releases what it held, so the peer receives physical keys.
- Key codes, typed text and window titles are never logged. `list_running_apps` returns exe names and titles only to the UI.
- Limitations:
  - Not validated against real games. Anti-cheat may ignore or flag injected input.
  - UIPI blocks hooks and injection into elevated windows unless TrapLink is elevated.
  - Logitech special buttons (gesture button, thumbwheel) and wheel or left/right buttons are not remappable.
  - Pause and Fn keys are not supported.
  - On macOS and Linux, remapping reports itself as unsupported.

## Optional account

Login is explicit: POST `/api/auth/device/code`, open its same-origin verification URL in the default browser, and poll `/api/auth/device/token` according to the server interval. After browser approval, the short-lived token registers the installation at `/api/v1/devices/register`. Only the resulting revocable device token is retained in the OS credential store. Logout revokes the registered device on the server before clearing it locally. No online request is needed for local device inventory, profiles, backups or LAN sharing. Explicit cloud backup upload/download uses Argon2id and XChaCha20-Poly1305 with a user-supplied passphrase; the server receives an encrypted envelope only. A download creates a new local backup and does not restore it automatically. See [cloud-backup.md](cloud-backup.md).

## Hardware support and remaining limits

Inventory classification never uses a name or manufacturer blacklist. A keyboard made by an audio brand is included when it exposes keyboard usage; a media-only device named "Keyboard" is excluded. The physical group comes from OS identity and vendor/product IDs. If physical identity cannot be resolved, exact HID paths remain separate instead of guessing from matching names or serial numbers. This conservative fallback can show multiple interfaces on a platform that does not expose grouping metadata.

`hidapi` enumerates currently present HID metadata. Only groups exposing keyboard or mouse collections appear; audio-only and media-only devices are filtered out. Windows groups by PnP ContainerId plus vendor/product, while macOS uses the physical IOKit ancestor with conservative identity fallbacks. Identical products with different physical identities stay separate. Inventory never opens a device or sends feature/output reports. Receiver-connected devices may expose only the receiver metadata. An explicit settings query can inspect a directly connected Logitech HID++ keyboard; Fn inversion is enabled only after verifying the relevant feature and its current state. Writes require the user's toggle and confirmed device readback. Receiver slots are never guessed. See [hidpp.md](hidpp.md) for exact support and protocol tests. DPI, lighting, onboard memory and on-device button remapping are not implemented; per-application remapping is done in software on Windows (see Remapping) and never writes to the device. Start on login, background tray operation, continuous clipboard sync and Internet relays are also absent.

The native test suite exercises storage validation, path/frame bounds, invitation parsing and expiration, rejection of malformed/nonlocal invitation destinations, candidate fallback with receiver identity pinning, Noise tamper rejection, cleanup of cancelled pairing handshakes and two independent local TCP peers that pair from one invitation, reject reuse, deny ungranted transfer, transfer an encrypted multichunk file, and reconnect with pinned identity. Flow tests cover:
- reconnect backoff bounds, jitter and give-up, the tie-break and resume detection
- edge pressure versus sliding
- outer edges and seams in negative-coordinate multi-monitor layouts
- entry mapping and `SendInput` absolute normalization
- hook-health detection with cooldown
- start-error classification
- a randomized table test proving the switch state machine always returns to `Local`

This validates the transport on one machine; it does not establish real two-machine firewall behavior or macOS hardware/permission compatibility. Native file selection uses `rfd` through narrow Rust commands; the webview has no generic filesystem or URL-opening permission.
