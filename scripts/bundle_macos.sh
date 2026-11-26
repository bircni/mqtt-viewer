#!/usr/bin/env bash
#
# Builds "MQTT Viewer.app" for Intel and Apple Silicon and packs it into a disk image in dist/. The Publish
# workflow builds on Linux and cannot do this, so the image is built on a Mac and attached to the release by hand:
#
#     gh release upload <version> dist/mqtt-viewer-<version>-universal-apple-darwin.dmg
#
# The app is signed ad hoc, not with a Developer ID, so macOS asks for confirmation on the first start.
#
# Usage: scripts/bundle_macos.sh

set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$(uname -s)" != "Darwin" ]; then
    echo "This script needs macOS." 1>&2
    exit 1
fi

targets=(aarch64-apple-darwin x86_64-apple-darwin)
for target in "${targets[@]}"; do
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "The Rust target $target is missing. Install it with:" 1>&2
        echo "    rustup target add $target" 1>&2
        exit 1
    fi
done

# `cargo pkgid` prints something like "path+file:///…/apps/mqtt-viewer#1.0.0".
id="$(cargo pkgid -p mqtt-viewer)"
version="${id##*[#@]}"

binaries=()
for target in "${targets[@]}"; do
    cargo build --release --locked -p mqtt-viewer --target "$target"
    binaries+=("target/$target/release/mqtt-viewer")
done

app="dist/MQTT Viewer.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/mqtt-viewer" "${binaries[@]}"
cp assets/icon.icns "$app/Contents/Resources/icon.icns"
sed "s/@VERSION@/$version/g" assets/Info.plist > "$app/Contents/Info.plist"
codesign --force --sign - "$app"

# The image shows the app next to a link to the Applications folder, to drag one onto the other.
image="dist/mqtt-viewer-$version-universal-apple-darwin.dmg"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
rm -f "$image"
hdiutil create -quiet -volname "MQTT Viewer" -srcfolder "$staging" -format UDZO "$image"

echo "Built $app"
echo "Built $image"
