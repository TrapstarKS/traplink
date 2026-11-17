# Desktop UX audit (archived summary)

Correctness-focused review of the desktop UI (pairing, input permissions, identity, backup restore). Brand art was preserved; this pass was later followed by the shadcn redesign ([shadcn-desktop-audit.md](shadcn-desktop-audit.md)). No quality score was awarded because the final visual review was not completed.

## Fixed defects

| Defect | Location | Fix |
|---|---|---|
| Two same-frame clicks could send duplicate native writes | `src/lib/use-desktop.ts` | Synchronous action ownership blocks duplicates, with an emergency-stop escape hatch |
| A pre-write read could leave stale values after success | `src/lib/use-desktop.ts` | Post-write refresh waits for the old read, then fetches a new snapshot |
| Background refresh or navigation discarded edited preferences | `App.tsx`, `Preferences.tsx` | Settings page stays mounted with a dirty baseline; saved values become the new baseline |
| Cloud help pointed to a nonexistent Account sub-tab | `CloudBackup.tsx` | Direct "Open my account" navigation |
| Frontend limits differed from native validation | `Backups.tsx`, `Profiles.tsx`, `CloudBackup.tsx` | 80-char backup name, 80-byte mapping fields, 12 code-point / 1024-byte passphrase |
| Expired invitation still looked valid | `Connections.tsx` | Live expiry label, copy disabled when expired, invite cancelled on close |
| Cancelled login could be revived by a late response | `Account.tsx` | Generation guard plus native `cancel_login`; pending login survives navigation |
| Raw HID interfaces counted as devices (33 rows, duplicates, microphone) | Devices inventory | Native code classifies and groups physical keyboards/mice; UI uses only `inputKinds` |
| Fn state could be requested twice or go stale after a failed write | `Devices.tsx` | Synchronous probe guard; errors clear state and require a fresh read |
| Per-peer copy said "receive" while native checked both directions | `Connections.tsx` | Copy describes sharing; actions follow the same per-peer prerequisites |
| Known native errors lacked recovery guidance | `src/lib/bridge.ts` | Known errors mapped to localized actions; unknown errors shown verbatim |
| A failed snapshot fetch rendered an empty workspace | `App.tsx` | Distinct failed-read state with retry |
| Cursor, motion and color token issues | `styles.css`, `ui/tabs.tsx` | Explicit cursor, no card lift, scoped transitions, art colors moved to tokens |

Device classification was later hardened: no manufacturer-name heuristics, separate identical products stay separate, and unresolved identities keep exact HID paths separate (41 native tests at the time).

## Gates

Copy, chrome-noise, stroke and token detectors went to zero; TypeScript and the Vite production build passed.

## Not verified

- Final screenshots, 390 px overflow, light/system theme and worst-case text in every modal.
- Two-computer input and pair/send/receive on physical machines, real Fn writes, Bluetooth reconnect, OS Accessibility prompts.

## Follow-up idea

Error translation depends on native English strings; a typed error-code contract would be more durable.
