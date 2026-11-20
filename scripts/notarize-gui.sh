#!/usr/bin/env bash
# Sign build/BerthGui.app with a Developer ID and notarize it.
#
# Needs a paid Apple Developer account with a "Developer ID Application"
# certificate in the login keychain, plus notarytool credentials stored once:
#
#   xcrun notarytool store-credentials berth-notary \
#     --apple-id <apple-id> --team-id <team-id> --password <app-specific-password>
#
# Then:
#   DEVELOPER_ID="Developer ID Application: Name (TEAMID)" \
#   NOTARY_PROFILE=berth-notary make notarize
#
# Without the certificate this exits with a clear message; `make gui-app`
# stays ad-hoc signed for local use.
set -euo pipefail

cd "$(dirname "$0")/.."
APP="build/BerthGui.app"
ZIP="build/BerthGui-notarized.zip"

if [[ ! -d "$APP" ]]; then
  echo "notarize: $APP is missing (run: make gui-app)" >&2
  exit 1
fi

if [[ -z "${DEVELOPER_ID:-}" ]]; then
  echo "notarize: set DEVELOPER_ID to the 'Developer ID Application: Name (TEAMID)' identity" >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -qF "$DEVELOPER_ID"; then
  echo "notarize: no such signing identity in the keychain: $DEVELOPER_ID" >&2
  echo "notarize: install the Developer ID Application certificate first (Xcode > Settings > Accounts)" >&2
  exit 1
fi

if [[ -z "${NOTARY_PROFILE:-}" && -z "${NOTARY_APPLE_ID:-}" ]]; then
  echo "notarize: set NOTARY_PROFILE (notarytool keychain profile) or NOTARY_APPLE_ID, NOTARY_TEAM_ID and NOTARY_PASSWORD" >&2
  exit 1
fi

echo "==> signing nested binaries"
for nested in "$APP/Contents/MacOS/berth-gui" "$APP/Contents/MacOS/berth"; do
  [[ -f "$nested" ]] || continue
  codesign --force --timestamp --options runtime --sign "$DEVELOPER_ID" "$nested"
done

echo "==> signing $APP"
codesign --force --timestamp --options runtime --sign "$DEVELOPER_ID" "$APP"
codesign --verify --deep --strict "$APP"

echo "==> submitting to notarytool"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
  xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
else
  xcrun notarytool submit "$ZIP" \
    --apple-id "${NOTARY_APPLE_ID}" \
    --team-id "${NOTARY_TEAM_ID}" \
    --password "${NOTARY_PASSWORD}" \
    --wait
fi

echo "==> stapling"
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose=4 "$APP"
echo "ok: $APP notarized and stapled"
