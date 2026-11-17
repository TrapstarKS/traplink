# TrapLink self-hosted runners

Release builds run **only** on the maintainer's machines (a Windows PC and an Apple Silicon Mac), registered as self-hosted runners for this repository. No job uses GitHub-hosted runners, Actions cache or workflow artifacts. Installers are uploaded straight to the GitHub Release with `gh`; Release assets do not count against Actions storage quota.

## Repository configuration

The target repository is `TrapstarKS/traplink`. To change it, edit:

- `apps/desktop/src-tauri/tauri.conf.json` → `plugins.updater.endpoints` (the `latest.json` feed used by installed apps);
- the default `-Repository` parameter of `setup-windows-runner.ps1` and the `REPOSITORY` variable in `setup-mac.sh`.

The workflows and `scripts/release/release.mjs` use `github.repository` automatically.

## Security model

- **Tags or manual runs only.** `desktop.yml` triggers on `push` of `v*` tags and on `workflow_dispatch`. There is no `pull_request` or `pull_request_target`, so fork code never reaches your machines. Only users with write access can create tags or dispatch workflows.
- **Fork approval.** In Settings → Actions → General → "Fork pull request workflows from outside collaborators", choose **"Require approval for all outside collaborators"**. This is a second barrier in case a `pull_request` workflow is added by mistake.
- **Dedicated label.** Jobs request `[self-hosted, <OS>, <arch>, traplink]`. A workflow that asks only for `self-hosted` will not land on these runners unless it uses the label.
- **Low-privilege account.** On Windows the service runs as `NT AUTHORITY\NETWORK SERVICE`, with no access to your profile, keys or browser. On the Mac, use a dedicated standard (non-admin) user.
- **Secrets.** Repository secrets are never passed to fork-triggered workflows. `GITHUB_TOKEN` has `contents: write` only in release jobs, and checkout does not persist credentials (`persist-credentials: false`).
- **No cache.** No `actions/cache`, `cache: pnpm` or `rust-cache`. The `cargo-target` folder stays on the runner's local disk next to the checkout, only to speed up later builds.

## Update signing key

The Tauri updater installs only packages signed by the private key matching the public key in `tauri.conf.json`. The private key lives **outside the repository** at `~/.traplink/updater/traplink.key` (no password), with the public key next to it in `traplink.key.pub`.

```powershell
# PowerShell does not support "<"; cmd passes the file through unchanged.
cmd /c "gh secret set TAURI_SIGNING_PRIVATE_KEY --repo TrapstarKS/traplink < %USERPROFILE%\.traplink\updater\traplink.key"
```

Back the file up securely (password manager or encrypted offline media). If the key is lost, installed apps will reject new versions and must be reinstalled manually. Never paste it into issues, logs or commits.

## macOS code-signing certificate

macOS builds are signed with one stable self-signed certificate (`TrapLink Self-Signed Code Signing`, SHA-1 `d934ec229fc79817f099c49d2d69fe41dba7fc18`, valid until 2046). Because the signature identity never changes, macOS keeps Accessibility and Input Monitoring grants across updates. The certificate, private key and PKCS#12 export live **outside the repository** in `~/.traplink/macsign/` (`traplink-macsign.p12`, `password.txt`, `key.pem`, `cert.pem`) and are stored as the secrets `MAC_CERT_P12` (base64 of the `.p12`) and `MAC_CERT_PASSWORD`.

```powershell
cmd /c "gh secret set MAC_CERT_P12 --repo TrapstarKS/traplink < %USERPROFILE%\.traplink\macsign\traplink-macsign.p12.b64"
cmd /c "gh secret set MAC_CERT_PASSWORD --repo TrapstarKS/traplink < %USERPROFILE%\.traplink\macsign\password.txt"
```

The Mac runner is a non-admin daemon user, so `codesign` cannot use a keychain identity there. `scripts/release/macos-signing.sh` downloads a pinned, checksum-verified `rcodesign`, places a `codesign` shim first in `PATH` for the Tauri build so signing uses the `.p12` directly, verifies `TrapLink.app` (`codesign --verify --strict --deep` and a designated requirement containing the certificate hash) and deletes everything afterwards. The release fails instead of shipping an ad-hoc build. Back the folder up like the updater key: losing it means a new certificate, and every Mac user would have to grant the permissions again once.

## Windows (PC)

Install prerequisites **machine-wide**, because the service cannot see your profile. In an elevated PowerShell:

```powershell
winget install Git.Git --scope machine
winget install GitHub.cli --scope machine
winget install OpenJS.NodeJS.LTS --scope machine   # Node 22
npm install -g --prefix "$env:ProgramFiles\nodejs" pnpm@10.10.0
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

### Machine-wide Rust

Default `rustup` installs into your profile. For the service, install into `C:\rust`:

```powershell
[Environment]::SetEnvironmentVariable("RUSTUP_HOME", "C:\rust\rustup", "Machine")
[Environment]::SetEnvironmentVariable("CARGO_HOME", "C:\rust\cargo", "Machine")
$env:RUSTUP_HOME = "C:\rust\rustup"; $env:CARGO_HOME = "C:\rust\cargo"
Invoke-WebRequest https://win.rustup.rs/x86_64 -OutFile "$env:TEMP\rustup-init.exe"
& "$env:TEMP\rustup-init.exe" -y --no-modify-path --profile minimal --default-toolchain stable
$path = [Environment]::GetEnvironmentVariable("Path", "Machine")
[Environment]::SetEnvironmentVariable("Path", "$path;C:\rust\cargo\bin", "Machine")
icacls C:\rust /grant "NETWORK SERVICE:(OI)(CI)M" /T
```

The workflow also adds `C:\rust\cargo\bin` to the job `PATH`. WebView2 ships with Windows 11; Tauri downloads NSIS on the first build.

### Register the runner

```powershell
$token = gh api -X POST repos/TrapstarKS/traplink/actions/runners/registration-token --jq .token
powershell -ExecutionPolicy Bypass -File .\scripts\runner\setup-windows-runner.ps1 -Token $token
```

The script checks for elevation, downloads the latest `actions/runner` (win-x64), verifies the SHA-256 published in the release notes, extracts to `C:\actions-runner-traplink` and installs the service with `--runasservice`. After changing the system `PATH`, restart the service: `Get-Service actions.runner.* | Restart-Service`.

## macOS (Apple Silicon)

One script does everything. In Terminal, as your admin user and without `sudo`:

```sh
curl -fsSL https://raw.githubusercontent.com/TrapstarKS/traplink/main/scripts/runner/setup-mac.sh -o setup-mac.sh
bash setup-mac.sh
```

It:
- installs Xcode Command Line Tools if missing;
- creates a hidden, standard, non-admin `traplink-runner` user (random password that is not stored);
- installs Rust, Node 22, pnpm 10.10.0 and GitHub CLI into that user's home only, verifying checksums (no Homebrew);
- downloads the official runner, verifies its SHA-256, registers it with the `traplink` label (the registration token is read without echo) and installs a LaunchDaemon that starts at boot without anyone logged in, running as `traplink-runner`;
- installs or updates TrapLink in `/Applications` from the latest release that has a Mac installer.

Other options: `--app-only`, `--runner-only` and `--remove-runner`. The script is safe to re-run.

## Publishing a release

1. Set the same version in `apps/desktop/src-tauri/tauri.conf.json`, `apps/desktop/src-tauri/Cargo.toml` and `apps/desktop/package.json` (the `prepare` job rejects mismatches).
2. `git tag v0.1.0 && git push origin v0.1.0`.
3. The workflow creates a draft Release, each runner uploads its files, and the final job builds `latest.json` and publishes. Edit the draft notes before it finishes if you like; they become the notes shown in the app.

To prune old releases: Actions → "Prune old releases" (dry run by default), or `node scripts/release/release.mjs prune --keep 3 --yes` with `GITHUB_REPOSITORY` set and `gh` authenticated.

## Removing a runner

- Windows (elevated PowerShell): `cd C:\actions-runner-traplink; .\config.cmd remove --token <token>`
- macOS: `cd ~/actions-runner-traplink && ./svc.sh stop && ./svc.sh uninstall && ./config.sh remove --token <token>`

Removal token: `gh api -X POST repos/TrapstarKS/traplink/actions/runners/remove-token --jq .token`. You can also remove runners in Settings → Actions → Runners.
