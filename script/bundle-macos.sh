#!/usr/bin/env bash
# Wraps the `diskmap` binary in a macOS .app so Finder and the Dock show the app icon.
# Usage: script/bundle-macos.sh [debug|release]   (default: release; builds first)
#        UNIVERSAL=1 script/bundle-macos.sh       (release for Apple silicon + Intel, joined with lipo)
set -euo pipefail
profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if [ "${UNIVERSAL:-0}" = 1 ]; then
  profile=release
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$t" >/dev/null
    cargo build -p diskmap-app --release --target "$t"
  done
  mkdir -p target/release
  lipo -create -output target/release/diskmap target/aarch64-apple-darwin/release/diskmap target/x86_64-apple-darwin/release/diskmap
elif [ "$profile" = release ]; then cargo build -p diskmap-app --release; else cargo build -p diskmap-app; fi
version="$(cargo pkgid -p diskmap-app | sed 's/.*[#@]//')"
app="target/$profile/diskmap.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$profile/diskmap" "$app/Contents/MacOS/diskmap"
cp crates/app/assets/logo/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>diskmap</string>
  <key>CFBundleDisplayName</key><string>diskmap</string>
  <key>CFBundleIdentifier</key><string>io.github.otherwisetech.diskmap</string>
  <key>CFBundleExecutable</key><string>diskmap</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
plutil -lint -s "$app/Contents/Info.plist"
# Make LaunchServices pick up the (possibly changed) icon.
touch "$app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app" || true
echo "$app"
