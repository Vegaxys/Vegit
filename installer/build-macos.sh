#!/bin/sh
# Builds Vegit.app for macOS in target/release/ and, with --install, copies it to /Applications.
#
#   sh installer/build-macos.sh            # build target/release/Vegit.app
#   sh installer/build-macos.sh --install  # build it and copy it to /Applications
#
# The icon is assets/brand/app-icon.icns, rendered from app-icon-macos.svg: the logo on Apple's
# continuous-corner shape (824 px in a 1024 canvas), so macOS 26+ shows it without a grey plate.
set -eu
cd "$(dirname "$0")/.."

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo build --release

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
APP=target/release/Vegit.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/vegit "$APP/Contents/MacOS/vegit"
cp assets/brand/app-icon.icns "$APP/Contents/Resources/vegit.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Vegit</string>
    <key>CFBundleDisplayName</key><string>Vegit</string>
    <key>CFBundleIdentifier</key><string>app.vegit.Vegit</string>
    <key>CFBundleExecutable</key><string>vegit</string>
    <key>CFBundleIconFile</key><string>vegit</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Ad-hoc signature: enough to run a locally built app on Apple Silicon.
codesign --force --deep --sign - "$APP"
echo "Built $APP"

if [ "${1:-}" = "--install" ]; then
    rm -rf /Applications/Vegit.app
    cp -R "$APP" /Applications/
    echo "Installed /Applications/Vegit.app"
fi
