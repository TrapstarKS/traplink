# Desktop shadcn redesign audit (archived summary)

## Scope

Replace the previous illustrated design with standard shadcn styling, as requested by the product owner. Changes were limited to desktop frontend source; native commands, Rust, shared contracts and dependencies were untouched. Verification was code and terminal only; no screen was opened.

## Changes

1. Connections: removed the illustrated network hero; now a peer card, a compact local-device card and transfer history.
2. Shell: 220 px sidebar and 56 px top bar instead of a 75 px top bar, expanded branding and decorative footer.
3. Global CSS: neutral/zinc semantic tokens and system UI fonts instead of custom blue surfaces and Space Grotesk.
4. Devices, Backups, Account: task titles and Card composition instead of marketing intros and decorative art.
5. Profiles, Preferences: consistent spacing with Cards, Fields, Tabs and Buttons.
6. Transfer status now recognizes both `complete` and `completed`; backup copy accurately describes what a restore changes and preserves.

All native event handlers and capability gates remain. Account and Preferences stay mounted during navigation to keep in-progress state. Invite expiry, login cancellation, restore/delete confirmations and the emergency stop are preserved.

## Review fixes

A read-only review against the previous commit found no lost handler or gate, but two layout regressions, both fixed in `styles.css`: dialogs needed a viewport height bound with scrolling, and long computer names needed truncation so the emergency stop stays reachable.

## Verification

TypeScript and Vite build passed; UI detectors and stroke scan reported zero findings; `git diff --check` passed; the Rust executable rebuilt with `--locked --features custom-protocol`.

## Not verified

Native screenshots, viewport overflow, keyboard walkthrough, two-computer pairing, cloud authorization and hardware writes.
