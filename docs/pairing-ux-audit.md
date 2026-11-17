# Pairing UX audit (archived summary)

Scope: `apps/desktop/src/pages/Connections.tsx`, a trust surface (permissions and pairing).

## Problem

The pairing dialog showed two separate values (address and secret) with a button copying both joined by a newline, while the receiving form needed two separate fields. A VPN address could be chosen simply by alphabetical order.

## Change

| Before | After | Benefit |
|---|---|---|
| Address field + secret field | One versioned invitation, parsed in Rust | One copy, one paste |
| "Copy address and code" | "Copy invitation" | Action matches the receiving field |
| First address alphabetically | Bounded set of private address candidates | Multi-network machines can try alternatives |
| Receiver types address and code | Receiver pastes one "Connection invitation" | Fewer transcription mistakes |

Existing Dialog, Tabs, Textarea, Field and Button primitives were reused. Expiry, cancellation, explicit sharing and default-off permissions are still enforced natively.

## Validation

- Frontend TypeScript and Vite build passed; source detectors reported zero findings.
- 44 native tests passed, covering round trip, malformed input and expiry rejection, pinned identity, endpoint fallback, replay denial and default permissions.
- Not done: final dialog pixel check and 390 px measurement (UI automation lacked input geometry).
