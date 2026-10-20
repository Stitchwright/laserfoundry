#!/usr/bin/env bash
# Imports a Developer ID Application certificate into a temporary keychain for code signing.
# Reads APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD and KEYCHAIN_PASSWORD from the environment and
# prints the keychain path on the last line of stdout. Delete it afterwards with `security delete-keychain <path>`.
# See https://developer.apple.com/documentation/security/creating-distribution-signed-code-for-the-mac
set -euo pipefail

: "${APPLE_CERTIFICATE:?}" "${APPLE_CERTIFICATE_PASSWORD:?}" "${KEYCHAIN_PASSWORD:?}"
dir="${RUNNER_TEMP:-$(mktemp -d)}"
keychain="$dir/laserfoundry-signing.keychain-db"
p12="$dir/certificate.p12"
trap 'rm -f "$p12"' EXIT

printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$p12"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$keychain"
security import "$p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
# Let codesign use the key without a prompt, and put the keychain first in the search list.
security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PASSWORD" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
security find-identity -v -p codesigning "$keychain" >&2
echo "$keychain"
