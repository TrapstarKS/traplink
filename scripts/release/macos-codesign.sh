#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
original=("$@")
sign=0
runtime=0
entitlements=""
targets=()
while [ $# -gt 0 ]; do
  case "$1" in
    -s|--sign) sign=1; shift 2 ;;
    --sign=*) sign=1; shift ;;
    --keychain) shift 2 ;;
    --entitlements) entitlements="$2"; shift 2 ;;
    --entitlements=*) entitlements="${1#--entitlements=}"; shift ;;
    -o|--options) case "$2" in *runtime*) runtime=1 ;; esac; shift 2 ;;
    --options=*) case "$1" in *runtime*) runtime=1 ;; esac; shift ;;
    -f|--force|--timestamp|--timestamp=*|--deep|-v|--verbose|--verbose=*) shift ;;
    -*) [ "$sign" = 1 ] && { echo "codesign shim: unsupported option $1" >&2; exit 1; }; shift ;;
    *) targets+=("$1"); shift ;;
  esac
done

if [ "$sign" = 0 ]; then
  exec /usr/bin/codesign "${original[@]}"
fi

[ "${#targets[@]}" -gt 0 ] || { echo "codesign shim: nothing to sign" >&2; exit 1; }
for target in "${targets[@]}"; do
  args=(sign --p12-file "$root/signing.p12" --p12-password-file "$root/password")
  [ "$runtime" = 1 ] && args+=(--code-signature-flags runtime)
  [ -n "$entitlements" ] && args+=(--entitlements-xml-file "$entitlements")
  "$root/bin/rcodesign" "${args[@]}" "$target"
done
