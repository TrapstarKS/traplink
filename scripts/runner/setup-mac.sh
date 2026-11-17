#!/bin/bash
set -Eeuo pipefail

REPOSITORY="${REPOSITORY:-TrapstarKS/traplink}"
RUNNER_USER="${RUNNER_USER:-traplink-runner}"
RUNNER_HOME="/Users/$RUNNER_USER"
RUNNER_DIR="$RUNNER_HOME/actions-runner-traplink"
RUNNER_LABELS="traplink"
RUNNER_NAME="${RUNNER_NAME:-$(scutil --get LocalHostName 2>/dev/null || hostname -s)-traplink}"
DAEMON_LABEL="com.traplink.actions-runner"
DAEMON_PLIST="/Library/LaunchDaemons/$DAEMON_LABEL.plist"
NODE_MAJOR="22"
PNPM_VERSION="10.10.0"
UA="traplink-mac-setup"
SIGNING_FINGERPRINT="d934ec229fc79817f099c49d2d69fe41dba7fc18"

usage() {
  cat <<'USAGE'
TrapLink Mac setup (Apple Silicon)

  bash setup-mac.sh                  install the app and register this Mac as a release runner
  bash setup-mac.sh --app-only       install or update the TrapLink app only
  bash setup-mac.sh --runner-only    register the release runner only
  bash setup-mac.sh --remove-runner  remove the release runner

The runner registration token (valid for 1 hour) is requested while the script runs,
or can be passed in RUNNER_TOKEN. Generate it with:
  gh api -X POST repos/TrapstarKS/traplink/actions/runners/registration-token --jq .token
USAGE
}

mode="all"
case "${1:-}" in
  "") ;;
  --app-only) mode="app" ;;
  --runner-only) mode="runner" ;;
  --remove-runner) mode="remove" ;;
  -h|--help) usage; exit 0 ;;
  *) echo "Unknown option: $1 (use --app-only, --runner-only or --remove-runner)" >&2; exit 1 ;;
esac

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
ok() { printf '    \033[32mok\033[0m %s\n' "$*"; }
warn() { printf '    \033[33mwarning\033[0m %s\n' "$*" >&2; }
die() { printf '\n\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
give() { sudo install -o "$RUNNER_USER" -m 600 "$1" "$RUNNER_HOME/.setup-$(basename "$1")"; printf '%s' "$RUNNER_HOME/.setup-$(basename "$1")"; }
as_runner() { sudo -u "$RUNNER_USER" -H env HOME="$RUNNER_HOME" PATH="$RUNNER_PATH" bash -c "$1"; }
api() { curl -fsSL -H "User-Agent: $UA" -H "Accept: application/vnd.github+json" "$1"; }

[ "$(uname -s)" = "Darwin" ] || die "this script is for macOS only."
[ "$(uname -m)" = "arm64" ] || die "this script requires an Apple Silicon Mac (M1 or newer)."
[ "$(id -u)" -ne 0 ] || die "run without sudo, as your own user. The script asks for your password when needed."

RUNNER_PATH="$RUNNER_HOME/.cargo/bin:$RUNNER_HOME/.local/node/bin:$RUNNER_HOME/.local/gh/bin:/usr/bin:/bin:/usr/sbin:/sbin"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"; [ -n "${mounted:-}" ] && hdiutil detach "$mounted" -quiet || true' EXIT
trap 'printf "\n\033[31merror:\033[0m failed at line %s: %s\n    Share a screenshot of these last lines.\n" "$LINENO" "$BASH_COMMAND" >&2' ERR

install_app() {
  step "TrapLink: looking up the latest release"
  local release url name version
  if ! release="$(api "https://api.github.com/repos/$REPOSITORY/releases/latest")"; then
    warn "no release has been published yet. Run again with --app-only after the first release."
    return 0
  fi
  version="$(printf '%s' "$release" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1 || true)"
  url="$(printf '%s' "$release" | grep -o '"browser_download_url": *"[^"]*_aarch64\.dmg"' | sed 's/.*"\(https[^"]*\)"/\1/' | head -n1 || true)"
  if [ -z "$url" ]; then
    warn "release $version has no Mac installer yet (Windows only)."
    warn "After this Mac's runner builds the next release, run: bash setup-mac.sh --app-only"
    return 0
  fi
  name="$(basename "$url")"

  local installed=""
  if [ -d /Applications/TrapLink.app ]; then
    installed="$(defaults read /Applications/TrapLink.app/Contents/Info CFBundleShortVersionString 2>/dev/null || true)"
  fi
  if [ -n "$installed" ] && [ "v$installed" = "$version" ]; then
    ok "TrapLink $version is already installed. Future releases arrive through automatic updates."
    return 0
  fi

  step "TrapLink: baixando $name"
  curl -fL --progress-bar -o "$tmp/$name" "$url"
  mounted="$(hdiutil attach -nobrowse -readonly -noautoopen "$tmp/$name" | awk -F'\t' '/\/Volumes\//{print $NF}' | head -n1 || true)"
  [ -d "$mounted/TrapLink.app" ] || die "the installer does not contain TrapLink.app."
  codesign --verify --deep "$mounted/TrapLink.app" 2>/dev/null || die "the downloaded app signature does not verify. Nothing was installed."
  local requirement
  requirement="$(codesign -d -r- "$mounted/TrapLink.app" 2>&1 | tr 'A-F' 'a-f' || true)"
  case "$requirement" in
    *"H\"$SIGNING_FINGERPRINT\""*) ok "signed with the TrapLink certificate ($SIGNING_FINGERPRINT)" ;;
    *) warn "this build is not signed with the TrapLink certificate. macOS may ask for Accessibility and Input Monitoring again after each update." ;;
  esac

  if pgrep -x TrapLink >/dev/null; then
    step "Closing the running TrapLink to update it"
    osascript -e 'quit app "TrapLink"' >/dev/null 2>&1 || true
    sleep 2
  fi
  step "Installing to /Applications"
  rm -rf /Applications/TrapLink.app 2>/dev/null || sudo rm -rf /Applications/TrapLink.app
  ditto "$mounted/TrapLink.app" /Applications/TrapLink.app 2>/dev/null || sudo ditto "$mounted/TrapLink.app" /Applications/TrapLink.app
  hdiutil detach "$mounted" -quiet || true
  mounted=""
  xattr -dr com.apple.quarantine /Applications/TrapLink.app 2>/dev/null || sudo xattr -dr com.apple.quarantine /Applications/TrapLink.app
  ok "TrapLink $version installed to /Applications/TrapLink.app"
  echo "    On first launch, macOS asks for Accessibility permission to control the mouse and keyboard."
  echo "    Updates: Preferences → Updates. The app also notifies you when a new release is out."
  open -a /Applications/TrapLink.app || true
}

remove_runner() {
  step "Removing this Mac's runner"
  sudo -v
  if [ -f "$DAEMON_PLIST" ]; then
    sudo launchctl bootout system "$DAEMON_PLIST" 2>/dev/null || true
    sudo rm -f "$DAEMON_PLIST"
    ok "service removed"
  fi
  if [ -f "$RUNNER_DIR/.runner" ]; then
    local token="${RUNNER_REMOVE_TOKEN:-}"
    if [ -z "$token" ]; then
      echo "    Removal token (generate it on the PC: gh api -X POST repos/$REPOSITORY/actions/runners/remove-token --jq .token)"
      printf "    Paste the token (or press Enter to remove it later in Settings → Actions → Runners): "
      read -rs token; echo
    fi
    if [ -n "$token" ]; then
      sudo -u "$RUNNER_USER" -H bash -c "cd '$RUNNER_DIR' && ./config.sh remove --token '$token'" || warn "could not unregister; remove it in Settings → Actions → Runners."
    fi
  fi
  echo "    To also delete the user and its tools: sudo sysadminctl -deleteUser $RUNNER_USER"
}

setup_runner() {
  step "Runner: checking prerequisites"
  if ! xcode-select -p >/dev/null 2>&1; then
    echo "    Installing the Xcode Command Line Tools (a window will open; click Install)..."
    xcode-select --install >/dev/null 2>&1 || true
    until xcode-select -p >/dev/null 2>&1; do sleep 10; done
  fi
  ok "Xcode Command Line Tools"

  local token="${RUNNER_TOKEN:-}"
  if [ ! -f "$RUNNER_DIR/.runner" ] && [ -z "$token" ]; then
    echo
    echo "    A runner registration token is required (valid for 1 hour). On the PC, run:"
    echo "      gh api -X POST repos/$REPOSITORY/actions/runners/registration-token --jq .token"
    printf "    Paste the token here (input is hidden): "
    read -rs token; echo
    [ -n "$token" ] || die "empty registration token."
  fi

  echo "    Your Mac password is requested now to create the user and the service."
  sudo -v
  while true; do sudo -n true; sleep 50; kill -0 "$$" 2>/dev/null || exit; done 2>/dev/null &

  step "Runner: dedicated user $RUNNER_USER"
  if id "$RUNNER_USER" >/dev/null 2>&1; then
    ok "already exists"
  else
    local password log
    password="$(openssl rand -hex 24)"
    [ -n "$password" ] || die "could not generate a random password for the user."
    log="$(sudo sysadminctl -addUser "$RUNNER_USER" -fullName "TrapLink Runner" -password "$password" -home "$RUNNER_HOME" 2>&1)" || true
    password=""
    id "$RUNNER_USER" >/dev/null 2>&1 || die "could not create user $RUNNER_USER. sysadminctl output: $log"
    sudo createhomedir -c -u "$RUNNER_USER" >/dev/null 2>&1 || true
    sudo dscl . -create "/Users/$RUNNER_USER" IsHidden 1 || warn "could not hide $RUNNER_USER from the login screen."
    ok "created (standard, non-admin, hidden from the login screen; the random password is not stored)"
  fi
  if dseditgroup -o checkmember -m "$RUNNER_USER" admin >/dev/null 2>&1; then
    die "user $RUNNER_USER is an administrator. Remove it from the admin group before continuing."
  fi
  sudo chmod 700 "$RUNNER_HOME"

  step "Runner: build tools (for user $RUNNER_USER only)"
  if as_runner 'command -v cargo' >/dev/null 2>&1; then
    as_runner 'rustup update stable --no-self-update >/dev/null 2>&1 || true'
  else
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$tmp/rustup-init.sh"
    local rustup_copy; rustup_copy="$(give "$tmp/rustup-init.sh")"
    as_runner "sh '$rustup_copy' -y --no-modify-path --profile minimal --default-toolchain stable >/dev/null && rm -f '$rustup_copy'"
  fi
  ok "$(as_runner 'rustc --version')"

  if ! as_runner "node --version 2>/dev/null | grep -q '^v$NODE_MAJOR\.'"; then
    local sums node_file
    sums="$(curl -fsSL "https://nodejs.org/dist/latest-v$NODE_MAJOR.x/SHASUMS256.txt")"
    node_file="$(printf '%s\n' "$sums" | awk '/node-v[0-9.]+-darwin-arm64\.tar\.gz$/{print $2; exit}')"
    [ -n "$node_file" ] || die "could not find Node $NODE_MAJOR for darwin-arm64."
    curl -fsSL -o "$tmp/$node_file" "https://nodejs.org/dist/latest-v$NODE_MAJOR.x/$node_file"
    [ "$(shasum -a 256 "$tmp/$node_file" | awk '{print $1}')" = "$(printf '%s\n' "$sums" | awk -v f="$node_file" '$2==f{print $1}')" ] || die "Node SHA-256 mismatch."
    local node_copy; node_copy="$(give "$tmp/$node_file")"
    as_runner "rm -rf ~/.local/node && mkdir -p ~/.local/node && tar -xzf '$node_copy' -C ~/.local/node --strip-components 1 && rm -f '$node_copy'"
  fi
  ok "Node $(as_runner 'node --version')"

  if ! as_runner "pnpm --version 2>/dev/null | grep -qx '$PNPM_VERSION'"; then
    as_runner "npm install -g --silent pnpm@$PNPM_VERSION >/dev/null"
  fi
  ok "pnpm $(as_runner 'pnpm --version')"

  if ! as_runner 'command -v gh' >/dev/null 2>&1; then
    local gh_release gh_version gh_zip gh_sums
    gh_release="$(api https://api.github.com/repos/cli/cli/releases/latest)"
    gh_version="$(printf '%s' "$gh_release" | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -n1 || true)"
    gh_zip="gh_${gh_version}_macOS_arm64.zip"
    curl -fsSL -o "$tmp/$gh_zip" "https://github.com/cli/cli/releases/download/v$gh_version/$gh_zip"
    gh_sums="$(curl -fsSL "https://github.com/cli/cli/releases/download/v$gh_version/gh_${gh_version}_checksums.txt")"
    [ "$(shasum -a 256 "$tmp/$gh_zip" | awk '{print $1}')" = "$(printf '%s\n' "$gh_sums" | awk -v f="$gh_zip" '$2==f{print $1}')" ] || die "GitHub CLI SHA-256 mismatch."
    local gh_copy; gh_copy="$(give "$tmp/$gh_zip")"
    as_runner "rm -rf ~/.local/gh ~/.local/gh-unzip && mkdir -p ~/.local/gh-unzip && ditto -x -k '$gh_copy' ~/.local/gh-unzip && mv ~/.local/gh-unzip/gh_* ~/.local/gh && rm -rf ~/.local/gh-unzip '$gh_copy'"
  fi
  ok "$(as_runner 'gh --version | head -n1')"

  step "Runner: GitHub Actions runner oficial"
  if [ -f "$RUNNER_DIR/.runner" ]; then
    ok "already registered in $RUNNER_DIR"
  else
    local runner_release version expected asset
    runner_release="$(api https://api.github.com/repos/actions/runner/releases/latest)"
    version="$(printf '%s' "$runner_release" | sed -n 's/.*"tag_name": *"v\([0-9.]*\)".*/\1/p' | head -n1 || true)"
    expected="$(printf '%s' "$runner_release" | grep -o 'BEGIN SHA osx-arm64 -->[[:space:]]*[0-9a-fA-F]\{64\}' | grep -o '[0-9a-fA-F]\{64\}' | head -n1 | tr 'A-F' 'a-f' || true)"
    [ -n "$version" ] && [ -n "$expected" ] || die "could not read the official runner version or SHA-256."
    asset="actions-runner-osx-arm64-$version.tar.gz"
    curl -fsSL -o "$tmp/$asset" "https://github.com/actions/runner/releases/download/v$version/$asset"
    [ "$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')" = "$expected" ] || die "runner SHA-256 mismatch. Nothing was installed."
    ok "runner v$version baixado, SHA-256 conferido"
    local runner_copy; runner_copy="$(give "$tmp/$asset")"
    as_runner "mkdir -p '$RUNNER_DIR' && tar -xzf '$runner_copy' -C '$RUNNER_DIR' && rm -f '$runner_copy'"
    printf '%s\n' "$token" | sudo -u "$RUNNER_USER" -H env HOME="$RUNNER_HOME" PATH="$RUNNER_PATH" bash -c \
      "cd '$RUNNER_DIR' && IFS= read -r t && ./config.sh --unattended --url 'https://github.com/$REPOSITORY' --token \"\$t\" --name '$RUNNER_NAME' --labels '$RUNNER_LABELS' --work _work --replace"
    token=""
    ok "registrado como $RUNNER_NAME (etiquetas: self-hosted, macOS, ARM64, $RUNNER_LABELS)"
  fi
  as_runner "printf '%s\n' '$RUNNER_PATH' > '$RUNNER_DIR/.path'"

  step "Runner: service that starts with the Mac"
  sudo tee "$DAEMON_PLIST" >/dev/null <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$DAEMON_LABEL</string>
  <key>UserName</key><string>$RUNNER_USER</string>
  <key>ProgramArguments</key><array><string>$RUNNER_DIR/run.sh</string></array>
  <key>WorkingDirectory</key><string>$RUNNER_DIR</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key><string>$RUNNER_HOME</string>
    <key>PATH</key><string>$RUNNER_PATH</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>$RUNNER_DIR/_diag/service.log</string>
  <key>StandardErrorPath</key><string>$RUNNER_DIR/_diag/service.log</string>
</dict>
</plist>
PLIST
  sudo chown root:wheel "$DAEMON_PLIST"
  sudo chmod 644 "$DAEMON_PLIST"
  as_runner "mkdir -p '$RUNNER_DIR/_diag'"
  sudo launchctl bootout system "$DAEMON_PLIST" 2>/dev/null || true
  sudo launchctl bootstrap system "$DAEMON_PLIST"
  sudo launchctl enable "system/$DAEMON_LABEL"
  sleep 3
  local daemon_state; daemon_state="$(sudo launchctl print "system/$DAEMON_LABEL" 2>/dev/null || true)"
  if [[ "$daemon_state" == *"state = running"* ]]; then
    ok "runner running (starts automatically when the Mac boots)"
  else
    warn "the service does not appear to be running. See $RUNNER_DIR/_diag/service.log"
  fi
  echo
  echo "    On the PC, to include the Mac in future releases:"
  echo "      gh variable set RELEASE_PLATFORMS --body windows,macos --repo $REPOSITORY"
}

case "$mode" in
  all) setup_runner; install_app ;;
  app) install_app ;;
  runner) setup_runner ;;
  remove) remove_runner ;;
esac
step "Done"
