# Website shadcn restyle audit (archived summary)

## Scope

Restyle the authentication website (sign-in, sign-up, device authorization, completion, privacy, not-found) to standard new-york/zinc shadcn with a compact brand mark and a centered 400 px form card, as requested by the product owner. Server APIs, auth/security libraries, credential storage, SMTP and password recovery were out of scope. Stack: Next.js 15.5.29, React 18.3.1, Tailwind 3, Radix shadcn.

## Design

- Zinc page, white card, near-black primary, muted zinc text and borders, semantic red errors.
- System sans; 24 px heading, 14 px body and labels.
- Centered card, max 400 px wide (privacy uses `max-w-2xl`), 24 px padding and field-group gaps, 8 px label gaps.
- Reused installed Card, Button, Input, Field, Alert, Checkbox, Separator and InputGroup. No new dependency.
- Light theme by default; dark tokens available, no theme switcher.

## Changes

| Before | After |
|---|---|
| Split dark marketing panel plus unbounded form | One 400 px card on a muted zinc page |
| Illustrated monitors, orbit lines, dotted background, slogan | Small interlocking TrapLink mark with wordmark |
| Blue theme and custom display font | Zinc tokens and system sans |
| Marketing headings | Functional headings: "Sign in to your account", "Create an account", "Authorize app" |
| Unframed privacy article | Wider card with the same hierarchy |

No feature, field, consent, security gate or real device information was removed. Device verification still binds the normalized code to the session ID; `safeReturnTo` and callback handling are unchanged; consent and password visibility start off; completion follows only successful approve/deny calls.

## Verification

- `pnpm --filter @traplink/web typecheck`, `test` (8 security tests) and `build` passed.
- `git diff --check -- apps/web/src` passed; UI detectors and stroke scan reported zero findings.
- No changes under `apps/web/src/server`, `apps/web/src/lib` or `apps/web/src/app/api`.

## Not verified

Browser, mobile and keyboard checks, visual before/after frames and live state transitions (the owner requested terminal-only work).
