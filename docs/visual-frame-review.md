# Device list frame review (archived)

Review of a single **before** screenshot (991 × 703) of the Devices screen. No after frame or source was inspected.

## Observed problems

- Nine equal device cards: seven identical `HERO 68 HE` cards (`372e:103e`), one `Arctis Nova Pro Wireless` and one `fifine Microphone`. Nothing distinguished the duplicates.
- A microphone was offered as a device choice, contradicting the keyboard-and-mouse scope.
- Every card used the same generic USB icon and badge, with no keyboard/mouse distinction or connection state.

## Acceptance criteria for the fix

1. A keyboard/mouse-scoped list with input types visibly distinguished.
2. Audio-only and incompatible entries excluded.
3. One choice per physical device; legitimately distinct identical models visibly distinguished.
4. Visible indication that the list shows currently connected devices.
5. The list shown in the native desktop app.

The before frame failed criteria 1–3. The fix (native grouping and filtering) is described in [native.md](native.md); no after frame was reviewed here.
