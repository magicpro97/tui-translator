#!/usr/bin/env bash
# scripts/make-app-bundle.sh
# Wrap the already-built `target/release/tui-translator` binary into a minimal
# macOS .app bundle so screencapturekit's `@rpath/libswift_Concurrency.dylib`
# dependency resolves from `Contents/Frameworks/` (avoids the duplicate-class
# warning caused by macOS 26 injecting its own copy of that dylib into the
# process when the binary is launched bare from `target/release/`).
#
# Use case: local dev iteration on macOS. No rebuild, no codesign with
# Developer ID (ad-hoc signature only). For signed/notarized DMG artifacts
# use scripts/package-macos.sh instead.
#
# Usage:
#   ./scripts/make-app-bundle.sh
#
# Output:
#   dist/macos-aarch64-dev/tui-translator.app/
#
# The bundle inherits all features enabled in the source build (e.g.
# `cargo build --release --features release-macos-arm`); model weights are
# NOT bundled (downloaded on first run).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BINARY_NAME="tui-translator"
VERSION="$(grep '^version' "$REPO_ROOT/Cargo.toml" | head -1 | cut -d'"' -f2)"
BUNDLE_ID="com.tui-translator.app.dev"
SOURCE_BINARY="$REPO_ROOT/target/release/$BINARY_NAME"

DIST_DIR="$REPO_ROOT/dist/macos-aarch64-dev"
APP_BUNDLE="$DIST_DIR/$BINARY_NAME.app"
APP_CONTENTS="$APP_BUNDLE/Contents"
APP_MACOS="$APP_CONTENTS/MacOS"
APP_FRAMEWORKS="$APP_CONTENTS/Frameworks"
APP_RESOURCES="$APP_CONTENTS/Resources"

if [[ ! -x "$SOURCE_BINARY" ]]; then
  echo "ERROR: $SOURCE_BINARY not found or not executable." >&2
  echo "  Build it first:" >&2
  echo "    cargo build --release --features release-macos-arm --bin $BINARY_NAME" >&2
  exit 1
fi

# Locate libswift_Concurrency.dylib. The screencapturekit 6.1.0 crate (which
# needs macOS 13+ Swift Concurrency runtime) was linked against the
# CommandLineTools swift-5.5 macosx slice on this host. We bundle the same
# dylib so the .app loads it from Contents/Frameworks/ via @rpath instead of
# the conflicting system-injected copy.
SWIFT_CONCURRENCY_SRC="/Library/Developer/CommandLineTools/usr/lib/swift-5.5/macosx/libswift_Concurrency.dylib"
if [[ ! -f "$SWIFT_CONCURRENCY_SRC" ]]; then
  echo "ERROR: $SWIFT_CONCURRENCY_SRC not found." >&2
  echo "  Install Xcode Command Line Tools (or replace this path with the" >&2
  echo "  equivalent dylib from your local toolchain)." >&2
  exit 1
fi

echo "=== make-app-bundle (dev) ==="
echo "  version:  $VERSION"
echo "  source:   $SOURCE_BINARY"
echo "  output:   $APP_BUNDLE"
echo ""

rm -rf "$APP_BUNDLE"
mkdir -p "$APP_MACOS" "$APP_FRAMEWORKS" "$APP_RESOURCES"

# ── binary ────────────────────────────────────────────────────────────────────
cp "$SOURCE_BINARY" "$APP_MACOS/$BINARY_NAME"
chmod +x "$APP_MACOS/$BINARY_NAME"

# ── swift concurrency runtime ─────────────────────────────────────────────────
cp -L "$SWIFT_CONCURRENCY_SRC" "$APP_FRAMEWORKS/"

# Set rpath so the binary's @rpath/libswift_Concurrency.dylib resolves to
# Contents/Frameworks/. install_name_tool refuses to add a duplicate rpath,
# so remove any previous one first.
existing_rpaths="$(otool -l "$APP_MACOS/$BINARY_NAME" 2>/dev/null \
  | awk '/LC_RPATH/{flag=1; next} flag && /path /{print $2; flag=0}')"
for rpath in $existing_rpaths; do
  install_name_tool -delete_rpath "$rpath" "$APP_MACOS/$BINARY_NAME" 2>/dev/null || true
done
install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP_MACOS/$BINARY_NAME"

# ── Info.plist ────────────────────────────────────────────────────────────────
cat > "$APP_CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>           <string>$BINARY_NAME</string>
  <key>CFBundleIdentifier</key>           <string>$BUNDLE_ID</string>
  <key>CFBundleName</key>                 <string>tui-translator</string>
  <key>CFBundleDisplayName</key>          <string>tui-translator (dev)</string>
  <key>CFBundleVersion</key>              <string>$VERSION</string>
  <key>CFBundleShortVersionString</key>   <string>$VERSION</string>
  <key>CFBundlePackageType</key>          <string>APPL</string>
  <key>LSMinimumSystemVersion</key>       <string>13.0</string>
  <key>LSUIElement</key>                  <true/>
  <key>NSMicrophoneUsageDescription</key>
    <string>tui-translator needs microphone access to capture system audio for transcription.</string>
  <key>NSAppleEventsUsageDescription</key>
    <string>tui-translator may send Apple Events to helper processes.</string>
</dict>
</plist>
PLIST

# ── resources ────────────────────────────────────────────────────────────────
cp "$REPO_ROOT/LICENSE" "$APP_RESOURCES/LICENSE" 2>/dev/null || true
cp "$REPO_ROOT/README.md" "$APP_RESOURCES/README.md" 2>/dev/null || true
cp "$REPO_ROOT/config.example.json" "$APP_RESOURCES/config.example.json" 2>/dev/null || true

# Safety: model weights must not leak into a dev bundle either.
if find "$APP_BUNDLE" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' \
  -o -name '*.pt' -o -name '*.pth' \) | grep -q .; then
  echo "ERROR: model binary leaked into bundle. Aborting." >&2
  exit 1
fi

# ── ad-hoc sign (so macOS treats it as a real app, not a random Mach-O) ──────
echo "  Ad-hoc signing bundle…"
codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null 2>&1
codesign --verify --verbose=2 "$APP_BUNDLE" >/dev/null

echo ""
echo "=== bundle ready ==="
find "$APP_BUNDLE" -maxdepth 3 -type f | sort
echo ""
echo "Run from terminal with:"
echo "  $APP_BUNDLE/Contents/MacOS/$BINARY_NAME --print-system-info"
echo "Or launch the GUI launcher (needs a real TTY):"
echo "  open -a Terminal $APP_BUNDLE  # then run the binary path above"
echo ""
echo "Tip: symlink it into ~/Applications for Spotlight:"
echo "  ln -sf \"$APP_BUNDLE\" ~/Applications/"
