#!/usr/bin/env bash
# Builds Velvet Hammer as a local macOS app, "Velvet Hammer.app", and installs it into
# /Applications, or into the folder given. Run it again to update the app in place: quit Velvet
# Hammer first, or quit and reopen it afterwards to run the new build.
# Usage: scripts/install-app.sh [DEST_DIR]   (default: /Applications)
#
# The bundle is signed ad hoc on this Mac, not with a developer ID, and is not notarized. That is
# enough for macOS to post its notifications; see "Install the app" in the README for the first
# launch.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
dest=${1:-/Applications}
name="Velvet Hammer"
id=io.github.vinspee.velvet-hammer
icon=$root/assets/AppIcon.icns

if [ "$(uname -s)" != Darwin ]; then
  echo "install-app.sh builds a macOS app; this is $(uname -s)." >&2
  exit 1
fi
if [ ! -d "$dest" ]; then
  echo "$dest is not a folder." >&2
  exit 1
fi

cargo build --release --locked --manifest-path "$root/Cargo.toml"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
build=$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 1)

# Built beside the target folder, then moved into place whole.
stage=$root/target/bundle
app=$stage/$name.app
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/velvet-hammer" "$app/Contents/MacOS/velvet-hammer"

icon_entry=
if [ -f "$icon" ]; then
  cp "$icon" "$app/Contents/Resources/AppIcon.icns"
  icon_entry="<key>CFBundleIconFile</key><string>AppIcon</string>"
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleExecutable</key><string>velvet-hammer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$build</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  $icon_entry
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Ad hoc: signed for this Mac only. Local notifications need no entitlement.
codesign --force --sign - --identifier "$id" "$app"
codesign --verify --strict "$app"

target=$dest/$name.app
rm -rf "$target"
ditto "$app" "$target"
# A bundle built here carries no quarantine flag; this clears one a copy may have picked up.
xattr -dr com.apple.quarantine "$target" 2>/dev/null || true
# Tells Launch Services about the bundle now, so Spotlight and notification clicks find it at once.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$target"

echo "Installed $target ($version, build $build)."
