#!/usr/bin/env bash
# Assembles LaserFoundry.app from the release binaries, then (optionally) signs, notarizes and staples it.
#
#   package.sh <binaries-dir> <out-dir>                 unsigned app, for a local look
#   KEYCHAIN=<path> APPLE_ID=... APPLE_PASSWORD=... APPLE_TEAM_ID=... package.sh <binaries-dir> <out-dir>
#
# <binaries-dir> holds `laserfoundry` and `laserfoundry-cli`. <out-dir> receives laserfoundry-macos-arm64.zip with
# LaserFoundry.app, the signed command-line tool and the README. Signing uses the "Developer ID Application" identity with
# the hardened runtime and a secure timestamp, as notarization requires:
# https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
set -euo pipefail

bin="$(cd "${1:?binaries dir}" && pwd)"
out="$(mkdir -p "${2:?output dir}" && cd "$2" && pwd)"
root="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\([0-9]*\.[0-9]*\.[0-9]*\).*"/\1/p;}' "$root/Cargo.toml" | head -1)"
[ -n "$version" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="$work/LaserFoundry.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
/bin/cp -f "$bin/laserfoundry" "$app/Contents/MacOS/laserfoundry"
/bin/cp -f "$bin/laserfoundry-cli" "$work/laserfoundry-cli"
/bin/cp -f "$root/README.md" "$work/README.md"
chmod +x "$app/Contents/MacOS/laserfoundry" "$work/laserfoundry-cli"
# The icon is drawn in code (crates/icon); it is not an image file in the repository.
(cd "$root" && cargo run --release --locked -q -p laserfoundry-icon -- icns "$app/Contents/Resources/AppIcon.icns")

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>LaserFoundry</string>
  <key>CFBundleDisplayName</key><string>LaserFoundry</string>
  <key>CFBundleIdentifier</key><string>app.stitchfoundry.laserfoundry</string>
  <key>CFBundleExecutable</key><string>laserfoundry</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

if [ -n "${KEYCHAIN:-}" ]; then
  : "${APPLE_ID:?}" "${APPLE_PASSWORD:?}" "${APPLE_TEAM_ID:?}"
  sign() { codesign --force --timestamp --options runtime --keychain "$KEYCHAIN" --sign "Developer ID Application" "$@"; }
  sign "$work/laserfoundry-cli"
  sign "$app"
  codesign --verify --strict --verbose=2 "$app"
  codesign --verify --strict --verbose=2 "$work/laserfoundry-cli"

  # Notarize everything that ships (the app and the loose tool) in one submission, then staple the app.
  mkdir -p "$work/submit" && /bin/cp -Rf "$app" "$work/laserfoundry-cli" "$work/submit/"
  ditto -c -k "$work/submit" "$work/notarize.zip"
  result="$(xcrun notarytool submit "$work/notarize.zip" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait --output-format json)"
  echo "$result"
  id="$(printf '%s' "$result" | sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' | head -1)"
  if ! printf '%s' "$result" | grep -q '"status" *: *"Accepted"'; then
    [ -z "$id" ] || xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" >&2 || true
    echo "notarization was not accepted" >&2
    exit 1
  fi
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  spctl --assess --type execute --verbose=2 "$app"
fi

ditto -c -k "$work" "$out/laserfoundry-macos-arm64.zip"
echo "wrote $out/laserfoundry-macos-arm64.zip"
