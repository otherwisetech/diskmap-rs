#!/usr/bin/env bash
# Wraps the `diskmap` binary in a macOS .app so Finder and the Dock show the app icon.
# Usage: script/bundle-macos.sh [debug|release]   (default: release; builds first)
#        UNIVERSAL=1 script/bundle-macos.sh       (release for Apple silicon + Intel, joined with lipo)
# Honors CARGO_TARGET_DIR (the .app is written next to the binary).
set -euo pipefail
profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-target}"
if [ "${UNIVERSAL:-0}" = 1 ]; then
  profile=release
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$t" >/dev/null
    cargo build -p diskmap-app --release --target "$t"
  done
  mkdir -p "$target/release"
  lipo -create -output "$target/release/diskmap" "$target/aarch64-apple-darwin/release/diskmap" "$target/x86_64-apple-darwin/release/diskmap"
elif [ "$profile" = release ]; then cargo build -p diskmap-app --release; else cargo build -p diskmap-app; fi
version="$(cargo pkgid -p diskmap-app | sed 's/.*[#@]//')"
app="$target/$profile/diskmap.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/$profile/diskmap" "$app/Contents/MacOS/diskmap"
cp crates/app/assets/logo/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
# Finder's "Analyze Space with diskmap" (NSServices below). Titles come from the app catalogs.
service_title() { sed -n 's/^app\.service\.analyze = //p' "crates/i18n/locales/$1/app.ftl"; }
service_default="$(service_title en)"
for lang in en es pt fr; do
  mkdir -p "$app/Contents/Resources/$lang.lproj"
  printf '"%s" = "%s";\n' "$service_default" "$(service_title "$lang")" > "$app/Contents/Resources/$lang.lproj/ServicesMenu.strings"
  plutil -lint -s "$app/Contents/Resources/$lang.lproj/ServicesMenu.strings"
done
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
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>es</string><string>pt</string><string>fr</string></array>
  <key>NSServices</key>
  <array>
    <dict>
      <key>NSMenuItem</key><dict><key>default</key><string>$service_default</string></dict>
      <key>NSMessage</key><string>analyzeFolder</string>
      <key>NSPortName</key><string>diskmap</string>
      <key>NSSendFileTypes</key><array><string>public.folder</string><string>public.volume</string></array>
      <!-- present = enabled by default instead of hidden in Keyboard settings -->
      <key>NSRequiredContext</key><dict><key>NSServiceCategory</key><string>public.folder</string></dict>
    </dict>
  </array>
  <!-- otherwise-diskmap:// links (otherwise-link: register::macos_url_types) -->
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>io.github.otherwisetech.diskmap</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>CFBundleURLSchemes</key><array><string>otherwise-diskmap</string></array>
    </dict>
  </array>
  <!-- Alternate, never Owner: Finder keeps opening folders; diskmap only shows up under "Open With" -->
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Folder</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key><array><string>public.folder</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
plutil -lint -s "$app/Contents/Info.plist"
# Make LaunchServices pick up the (possibly changed) icon.
touch "$app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app" || true
/System/Library/CoreServices/pbs -update || true
echo "$app"
