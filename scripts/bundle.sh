#!/usr/bin/env bash
# Build diffr.app for macOS: scripts/bundle.sh [--install]
# Writes target/release/diffr.app; --install also copies it to /Applications.
# Needs Xcode's Metal Toolchain (xcodebuild -downloadComponent MetalToolchain),
# rsvg-convert (brew install librsvg) and cargo-about (cargo install --locked cargo-about).
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/release/diffr.app
ICONSET=target/release/diffr.iconset

cargo build --release -p diffr

rm -rf "$APP" "$ICONSET"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ICONSET"
cp target/release/diffr "$APP/Contents/MacOS/diffr"

for s in 16 32 128 256 512; do
  rsvg-convert -w $s -h $s assets/icon.svg -o "$ICONSET/icon_${s}x${s}.png"
  rsvg-convert -w $((s * 2)) -h $((s * 2)) assets/icon.svg -o "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/diffr.icns"
rm -rf "$ICONSET"

# The licenses: diffr's, the notices for the works it contains, and the
# texts of every crate it links (cargo-about, see about.toml).
R=$APP/Contents/Resources
cp LICENSE NOTICE.md "$R/"
cp crates/app/LICENSE-icons "$R/LICENSE-lucide"
cp crates/fileicon/LICENSE-icons "$R/LICENSE-pierre-icons"
scripts/licenses.sh "$R/THIRD_PARTY_LICENSES.html"

cat >"$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>diffr</string>
  <key>CFBundleDisplayName</key><string>diffr</string>
  <key>CFBundleIdentifier</key><string>dev.diffr</string>
  <key>CFBundleExecutable</key><string>diffr</string>
  <key>CFBundleIconFile</key><string>diffr</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>CFBundleURLTypes</key>
  <array><dict>
    <key>CFBundleURLName</key><string>dev.diffr</string>
    <key>CFBundleURLSchemes</key><array><string>diffr</string></array>
  </dict></array>
</dict>
</plist>
EOF

# Ad-hoc signature: runs on this machine, not for distribution.
codesign --force --deep -s - "$APP"
echo "Built $APP"

if [ "${1:-}" = "--install" ]; then
  rm -rf /Applications/diffr.app
  cp -R "$APP" /Applications/diffr.app
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/diffr.app
  echo "Installed /Applications/diffr.app"
fi
