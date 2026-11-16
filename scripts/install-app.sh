#!/usr/bin/env bash
# Builds Velvet Hammer as a macOS app and installs or updates it at one fixed path,
# "~/Applications/Velvet Hammer Dev.app", so a Dock icon pinned to it always opens the newest build.
# Usage: scripts/install-app.sh
#
# Run it again to update. The swap goes through Rune's installer (scripts/mac-app in the Rune
# checkout Cargo already has): it waits while the chief of staff is in a turn, asks the running app
# to quit, exchanges the bundles atomically, opens the new build, and puts the previous bundle back
# when the new one is not ready within 15 seconds.
#
# The bundle is signed with $MAC_APP_SIGN when set, else with your "Apple Development" identity
# for the email in `git config user.email`, else ad hoc. A stable identity keeps the Dock icon and
# notification permission across rebuilds; ad hoc signing may ask for notification permission
# again after each build.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
name="Velvet Hammer"
file="Velvet Hammer Dev"
id=io.github.vinspee.velvet-hammer

if [ "$(uname -s)" != Darwin ]; then
  echo "install-app.sh builds a macOS app; this is $(uname -s)." >&2
  exit 1
fi

cargo build --release --locked --manifest-path "$root/Cargo.toml"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
build=$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 1)

# Rune's installer at the revision Cargo.lock pins: the checkout holding the `rune` crate.
rune=$(cargo metadata --format-version 1 --locked --manifest-path "$root/Cargo.toml" |
  python3 -c 'import json, sys
for p in json.load(sys.stdin)["packages"]:
    if p["name"] == "rune":
        print(p["manifest_path"].rsplit("/crates/", 1)[0])
        break')
installer=$rune/scripts/mac_app_install.py
[ -f "$installer" ] || {
  echo "Rune's installer is not at $installer." >&2
  exit 1
}

stage=$root/target/bundle
app=$stage/$file.app
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/velvet-hammer" "$app/Contents/MacOS/velvet-hammer"
cp "$root/assets/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"

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
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

identity=${MAC_APP_SIGN:-}
if [ -z "$identity" ]; then
  email=$(git -C "$root" config user.email || true)
  if [ -n "$email" ]; then
    identity=$(security find-identity -v -p codesigning |
      sed -n "s/.*\"\(Apple Development: $email ([A-Z0-9]*)\)\"/\1/p" | head -1)
  fi
fi
if [ -z "$identity" ]; then
  echo "No signing identity found; signing ad hoc. Set MAC_APP_SIGN to keep permissions across builds." >&2
  identity=-
fi
codesign --force --sign "$identity" --identifier "$id" "$app"
codesign --verify --strict "$app"

# Lets the running app quit for the swap without asking about workers in flight: they keep
# running, and the new build supervises them at once (see src/lifecycle.rs). A turn still holds
# the swap. The app honours the file only while this script runs.
data=${VELVET_HAMMER_DATA:-$HOME/.velvet-hammer}
mkdir -p "$data"
echo $$ > "$data/relaunch"
trap 'rm -f "$data/relaunch"' EXIT

python3 "$installer" --name "$file" --id "$id" --exe velvet-hammer --bundle "$app"

target=$HOME/Applications/$file.app
# A bundle built here carries no quarantine flag; this clears one a copy may have picked up.
xattr -dr com.apple.quarantine "$target" 2>/dev/null || true
# Tells Launch Services about the bundle now, so Spotlight and notification clicks find it at once.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$target"

if [ -d "/Applications/$name.app" ]; then
  echo "An older copy is at /Applications/$name.app with the same identifier; move it to the Trash so macOS opens this one." >&2
fi
echo "Installed $target ($version, build $build, signed by $identity)."
