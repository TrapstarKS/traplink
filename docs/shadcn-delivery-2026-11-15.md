# shadcn interface delivery, 2026-11-15 (archived summary)

Historical note; superseded by later releases.

- **Result.** Desktop app and website moved to stock shadcn components, zinc palette, system fonts and compact cards; the app keeps light and dark themes. The website remains authentication/authorization only. No native command, dependency or protocol changed. Also fixed: completed-transfer labels, exact restore description, long-dialog scrolling and truncation of long names in the top bar.
- **Package.** Unsigned Windows x64 test build (executable and ZIP under the local `artifacts` directory).
- **Verification.** Desktop TypeScript/Vite and Rust build passed; web TypeScript, 8 security tests and Next.js 15.5.29 build passed; UI detectors passed. Done from code and terminal only, with no visual validation.
- **Deployment.** Website updated by redeploying only the Compose `auth` service, keeping the previous image for rollback. Health and ten HTTP checks passed; other Nginx sites unchanged.
- **Open at the time.** macOS build and hardware testing; edge switching, automatic clipboard and cross-network relay were out of scope.
