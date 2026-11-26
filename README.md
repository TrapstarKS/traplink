# TrapLink

A desktop app for connecting computers and managing your setup on Windows and macOS. Local use needs no account. The website only handles sign-in, sign-up and app authorization; all management happens in the desktop app.

## Download

Get the latest build from [Releases](https://github.com/TrapstarKS/traplink/releases/latest): `TrapLink_<version>_x64-setup.exe` for Windows and `TrapLink_<version>_aarch64.dmg` for Apple Silicon Macs. Builds run on the maintainer's own machines (self-hosted runners), not on GitHub-hosted runners. See [scripts/runner](scripts/runner/README.md).

Installers are not code-signed yet. On Windows, SmartScreen may warn ("More info" → "Run anyway"). On macOS, open the app the first time with right-click → Open (or System Settings → Privacy & Security → Open Anyway).

## Automatic updates

The app checks for new versions at launch and notifies you. Nothing installs until you click "Download and install" in Preferences → Updates. Every package is signed and verified by the Tauri updater before installation; TrapLink restarts afterwards.

## Start at login and tray

"Start at login" in Preferences (off by default) registers TrapLink to launch at login, hidden in the tray. With it on, or while Flow or remapping is active, closing the window keeps the app in the tray (Open TrapLink, Flow and remapping status, Emergency stop (input and remapping), Quit). Launching the app again only shows the existing window. For safety, input sharing and Flow are off again whenever TrapLink exits.

## Layout

- `apps/desktop`: React 18 + Vite + shadcn/ui, running in Tauri 2 with a Rust core.
- `apps/web`: Next.js 15.5.29 + shadcn/ui, Better Auth and persistent SQLite for a single server.
- `packages/shared`: shared TypeScript contracts.
- `deploy`: Docker Compose, Caddy or Nginx with HTTPS, and database migration for a VPS.

## Development

Requirements: Node.js 22, pnpm 10.10, Rust and the Tauri platform prerequisites (Windows: C++ Build Tools and WebView2; macOS: Xcode Command Line Tools).

```sh
pnpm install
cp apps/web/.env.example apps/web/.env.local
# Generate BETTER_AUTH_SECRET and fill in the local file. Never commit secrets.
pnpm --filter @traplink/web db:migrate
pnpm dev:web
pnpm dev:app
```

In PowerShell, use `Copy-Item` instead of `cp`. Add `~/.cargo/bin` to `PATH` if Rust is not found.

Local website: `http://localhost:3100`. Desktop UI dev server (`pnpm dev:desktop`): `http://localhost:1420`. Opened in a plain browser, the desktop UI states that native features require the desktop app; it never shows fake connections or devices.

```sh
pnpm typecheck
pnpm test                          # web tests
pnpm --filter @traplink/web build
pnpm --filter @traplink/desktop build
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
pnpm build:app                     # native installer
```

## Local use

Open the app on each computer. On the receiving computer, create an invitation and click "Copy invitation"; paste it into "Use an invitation" on the other computer. Address and key travel together. Review the devices and grant only the permissions you want. Account connection and control permission are independent.

Invitations are short-lived secrets: share them directly with the person pairing and never publish them. A paired connection uses its own identity and authenticated encryption. See [docs/native.md](docs/native.md) for implemented features and current limits.

## Optional account

In Preferences, set your server URL and connect your account. The product default is `https://traplink.traphub.xyz`; in development use `http://localhost:3100`. The browser opens the authorization page; check the code before approving. The app receives its own revocable credential. The website has no device dashboard.

The server stores users, sessions and device credentials separately. Uploaded backups are encrypted envelopes. The Backups page can save a copy online and download it without restoring automatically. The backup passphrase is separate from the account password and is never stored. There is no password recovery and no SMTP requirement.

## Deployment

See [VPS deployment](deploy/README.md) and [Nginx mode](deploy/NGINX.md). The target is `traplink.traphub.xyz` on local port `3801` behind an existing Nginx; DNS must resolve before publishing. Desktop releases are published by pushing a `v<version>` tag, which runs `desktop.yml` on the self-hosted runners only (see [scripts/runner](scripts/runner/README.md)).

Windows signing, Apple signing/notarization and public distribution require the product owner's certificates. Before a public launch: validate between real machines, complete a security review, and publish operator privacy/retention information.

## Status

This is an early working product, not a promise of feature parity with Logi Options+. Hardware features are implemented per model and connection; a listed keyboard does not imply every setting is available. TrapLink does not depend on Options+ binaries or services.

There is no public relay yet. Connecting an account does not make computers behind NAT reachable; local transport requires direct connectivity between peers. See [docs/roadmap.md](docs/roadmap.md).
