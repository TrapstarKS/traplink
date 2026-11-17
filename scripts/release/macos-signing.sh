#!/bin/bash
set -euo pipefail

FINGERPRINT="d934ec229fc79817f099c49d2d69fe41dba7fc18"
RCODESIGN_VERSION="0.29.0"
RCODESIGN_SHA256="d98372d5524226ccf9dc0eda03d4e4f5826182dabb2fc3f2bd303ed9113a748d"
RCODESIGN_URL="https://github.com/indygreg/apple-platform-rs/releases/download/apple-codesign%2F$RCODESIGN_VERSION/apple-codesign-$RCODESIGN_VERSION-macos-universal.tar.gz"
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="${RUNNER_TEMP:?RUNNER_TEMP is required}/traplink-signing"

setup() {
  if [ -z "${MAC_CERT_P12:-}" ] || [ -z "${MAC_CERT_PASSWORD:-}" ]; then
    echo "::error::MAC_CERT_P12 and MAC_CERT_PASSWORD are required. Refusing to build an ad-hoc signed macOS app."
    exit 1
  fi
  umask 077
  rm -rf "$WORK"
  mkdir -p "$WORK/bin"
  curl -fsSL -o "$WORK/rcodesign.tar.gz" "$RCODESIGN_URL"
  echo "$RCODESIGN_SHA256  $WORK/rcodesign.tar.gz" | shasum -a 256 -c - >/dev/null
  tar -xzf "$WORK/rcodesign.tar.gz" -C "$WORK" --strip-components 1 "apple-codesign-$RCODESIGN_VERSION-macos-universal/rcodesign"
  mv "$WORK/rcodesign" "$WORK/bin/rcodesign"
  rm -f "$WORK/rcodesign.tar.gz"
  printf '%s' "$MAC_CERT_P12" | base64 --decode > "$WORK/signing.p12"
  printf '%s' "$MAC_CERT_PASSWORD" > "$WORK/password"
  local found
  found="$(openssl pkcs12 -in "$WORK/signing.p12" -passin "file:$WORK/password" -nokeys -clcerts 2>/dev/null | openssl x509 -noout -fingerprint -sha1 | sed 's/.*=//; s/://g' | tr 'A-F' 'a-f')"
  if [ "$found" != "$FINGERPRINT" ]; then
    echo "::error::MAC_CERT_P12 is not the TrapLink signing certificate ($FINGERPRINT)."
    exit 1
  fi
  cp "$HERE/macos-codesign.sh" "$WORK/bin/codesign"
  chmod 700 "$WORK/bin/codesign" "$WORK/bin/rcodesign"
  "$WORK/bin/rcodesign" --version
  echo "$WORK/bin" >> "${GITHUB_PATH:?GITHUB_PATH is required}"
  echo "macOS signing ready with certificate $FINGERPRINT."
}

verify() {
  local app="${1:?path to TrapLink.app is required}"
  [ -d "$app" ] || { echo "::error::$app was not built."; exit 1; }
  /usr/bin/codesign --verify --strict --deep --verbose=2 "$app"
  /usr/bin/codesign -dvv "$app" 2>&1 | grep -E '^(Identifier|Authority|TeamIdentifier|CodeDirectory)' || true
  local requirement
  requirement="$(/usr/bin/codesign -d -r- "$app" 2>&1 | sed -n 's/^#* *designated => *//p' | tr 'A-F' 'a-f')"
  echo "Designated requirement: ${requirement:-<none>}"
  case "$requirement" in
    *"certificate leaf = H\"$FINGERPRINT\""*|*"certificate root = H\"$FINGERPRINT\""*) ;;
    *) echo "::error::$app is not signed with the TrapLink certificate. macOS would drop Accessibility and Input Monitoring grants on update."; exit 1 ;;
  esac
  echo "$app is signed with the TrapLink certificate."
}

cleanup() {
  rm -rf "$WORK"
}

case "${1:-}" in
  setup) setup ;;
  verify) verify "${2:-}" ;;
  cleanup) cleanup ;;
  *) echo "usage: macos-signing.sh setup | verify <TrapLink.app> | cleanup" >&2; exit 1 ;;
esac
