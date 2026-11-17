# Website authentication UX audit (archived summary)

Scope: the Next.js 15.5.29 / React 18 / shadcn authentication website, a trust flow for identity and device permission. Goal: confirmed bug and accessibility fixes only, keeping the existing design, with no password recovery, SMTP, dashboard or API redesign. This pass was later followed by the shadcn restyle ([shadcn-web-audit.md](shadcn-web-audit.md)).

## Fixed findings

1. `connected.tsx`, `device-authorization.tsx`: `useSession().error` was ignored, so a failed session request looked signed out. Now a 401 leads to sign-in and network/server failures show a retry.
2. `connected.tsx`: sign-out redirected unconditionally with no pending or failure feedback. A rejected sign-out no longer looks successful.
3. `device-authorization.tsx`: no way to switch accounts. Added a switch-account action that preserves the device code.
4. `device-authorization.tsx`: four characters passed browser validation while the backend requires an eight-character normalized code; errors were detached from the input. Validation now matches and errors are field-linked.
5. `auth-form.tsx`: a query error pointed users at a recovery link even though recovery does not exist. Removed.
6. `auth-form.tsx`: no focus handoff or field-level error descriptions; a whitespace-only name could be submitted. Fixed.
7. `auth-form.tsx`: a hand-drawn shield duplicated the installed Lucide icons. Replaced.
8. Copy-lint fixes: decorative dashes and separators, an inline arrow and a semicolon.

Also: code verification is bound to the displayed session ID, so a changed session cannot keep a stale verified state. The privacy link from active forms opens in a new tab (announced to assistive technology) so unsent input is preserved.

Not changed without live evidence: illustration, typography, palette, spacing, analytics or indexing (the site is intentionally `noindex`).

## Verification

- `tsc --noEmit`, `next build` and Vitest (8/8) passed.
- UI detectors reported zero findings after the edits; one pre-existing illustration halo remains as retained art. No focus style was weakened.
- Separate production smoke checks passed: sign-up, secure session cookie, device approval/enrollment, backup CRUD and tenant isolation, revocation and origin rejection; disposable users were removed.

## Not verified

Rendering at mobile/desktop widths, live keyboard navigation, touch, browser errors, long names, reduced motion, and live approve/deny in the UI.
