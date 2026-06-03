#!/usr/bin/env bash
# REL-01: macOS release packaging — .app bundle, .dmg, notarization, and SHA256SUMS
#
# Usage:
#   ./scripts/package-macos.sh [--arch <x86_64|aarch64|universal>] [--sign] [--notarize]
#
# Required env for --sign / --notarize:
#   APPLE_DEVELOPER_ID_APP   — "Developer ID Application: Name (TEAM_ID)"
#   APPLE_DEVELOPER_ID_INST  — "Developer ID Installer: Name (TEAM_ID)"   (optional)
#   APPLE_KEYCHAIN_PROFILE   — keychain profile created with notarytool store-credentials
#   APPLE_BUNDLE_ID          — reverse-DNS bundle ID e.g. com.example.tui-translator
#
# Outputs (in ./dist/macos-<arch>/):
#   tui-translator.app      — application bundle (unsigned by default)
#   tui-translator.dmg      — disk image
#   SHA256SUMS              — checksums for all distributable files
#   *.log                   — verification logs (codesign, notarization, stapler, spctl)
#
# Notarization (--notarize) requires:
#   - Xcode command-line tools installed
#   - xcrun notarytool stored credentials (see APPLE_KEYCHAIN_PROFILE)
#   - Apple Developer Program membership

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── defaults ──────────────────────────────────────────────────────────────────
ARCH="${ARCH:-aarch64}"
DO_SIGN=false
DO_NOTARIZE=false
BINARY_NAME="tui-translator"
BUNDLE_ID="${APPLE_BUNDLE_ID:-com.tui-translator.app}"
VERSION="$(grep '^version' "$REPO_ROOT/Cargo.toml" | head -1 | cut -d'"' -f2)"
FEATURES="${RELEASE_FEATURES:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)     ARCH="$2";     shift 2 ;;
    --sign)     DO_SIGN=true;  shift   ;;
    --notarize) DO_NOTARIZE=true; DO_SIGN=true; shift ;;
    --features) FEATURES="$2"; shift 2 ;;
    *) echo "Unknown argument: $1"; exit 1 ;;
  esac
done

# Compute paths AFTER --arch parsing (otherwise --arch universal still uses the
# default 'aarch64' and writes DMG to dist/macos-aarch64/, leaving the workflow's
# dist/macos-universal/ glob empty).
DIST_DIR="$REPO_ROOT/dist/macos-$ARCH"
STAGING="$DIST_DIR/staging"

# ── resolve Rust target triple ─────────────────────────────────────────────────
case "$ARCH" in
  x86_64)   TARGET="x86_64-apple-darwin" ;;
  aarch64)  TARGET="aarch64-apple-darwin" ;;
  universal) TARGET="" ;;  # handled separately
  *) echo "ERROR: --arch must be x86_64, aarch64, or universal"; exit 1 ;;
esac

echo "=== REL-01 macOS packaging ==="
echo "  version:  $VERSION"
echo "  arch:     $ARCH"
echo "  target:   ${TARGET:-universal (lipo)}"
echo "  sign:     $DO_SIGN"
echo "  notarize: $DO_NOTARIZE"
echo ""

# ── build ──────────────────────────────────────────────────────────────────────
cd "$REPO_ROOT"

FEATURES_FLAG=""
if [[ -n "${FEATURES}" ]]; then
  FEATURES_FLAG="--features ${FEATURES}"
fi

if [[ "$ARCH" == "universal" ]]; then
  # shellcheck disable=SC2086
  cargo build --locked --release --target x86_64-apple-darwin --bin "$BINARY_NAME" ${FEATURES_FLAG}
  # shellcheck disable=SC2086
  cargo build --locked --release --target aarch64-apple-darwin --bin "$BINARY_NAME" ${FEATURES_FLAG}
  mkdir -p "$DIST_DIR"
  lipo -create \
    "target/x86_64-apple-darwin/release/$BINARY_NAME" \
    "target/aarch64-apple-darwin/release/$BINARY_NAME" \
    -output "$DIST_DIR/$BINARY_NAME-universal"
  BINARY_PATH="$DIST_DIR/$BINARY_NAME-universal"
  lipo -info "$BINARY_PATH" | tee "$DIST_DIR/lipo-info.log"
else
  # shellcheck disable=SC2086
  cargo build --locked --release --target "$TARGET" --bin "$BINARY_NAME" ${FEATURES_FLAG}
  BINARY_PATH="target/$TARGET/release/$BINARY_NAME"
fi

echo "Binary: $BINARY_PATH"

# ── .app bundle ───────────────────────────────────────────────────────────────
APP_BUNDLE="$DIST_DIR/$BINARY_NAME.app"
APP_CONTENTS="$APP_BUNDLE/Contents"
APP_MACOS="$APP_CONTENTS/MacOS"
APP_RESOURCES="$APP_CONTENTS/Resources"

rm -rf "$APP_BUNDLE"
mkdir -p "$APP_MACOS" "$APP_RESOURCES" "$STAGING"

cp "$BINARY_PATH" "$APP_MACOS/$BINARY_NAME"

# Bundle ORT runtime dylib if present (placed by ort/copy-dylibs feature).
# The dylib must be inside MacOS/ so macOS loader finds it relative to the binary.
ORT_DYLIB=""
for search_target in "x86_64-apple-darwin" "aarch64-apple-darwin"; do
  candidate="$(find "target/${search_target}/release" -maxdepth 1 \
    -name "libonnxruntime*.dylib" 2>/dev/null | head -1)"
  if [[ -n "${candidate}" ]]; then
    ORT_DYLIB="${candidate}"
    break
  fi
done
if [[ -n "${ORT_DYLIB}" ]]; then
  cp "${ORT_DYLIB}" "$APP_MACOS/"
  echo "  Bundled ORT runtime: $(basename "${ORT_DYLIB}")"
else
  echo "  NOTE: ORT dylib not found in build output (expected when using release-macos-* features)"
fi

cat > "$APP_CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>     <string>$BINARY_NAME</string>
  <key>CFBundleIdentifier</key>     <string>$BUNDLE_ID</string>
  <key>CFBundleName</key>           <string>tui-translator</string>
  <key>CFBundleVersion</key>        <string>$VERSION</string>
  <key>CFBundleShortVersionString</key> <string>$VERSION</string>
  <key>CFBundlePackageType</key>    <string>APPL</string>
  <key>LSMinimumSystemVersion</key> <string>12.0</string>
  <key>LSUIElement</key>            <true/>
  <key>NSMicrophoneUsageDescription</key>
    <string>tui-translator needs microphone access to capture system audio for transcription.</string>
  <key>NSAppleEventsUsageDescription</key>
    <string>tui-translator may send Apple Events to helper processes.</string>
</dict>
</plist>
PLIST

# Copy LICENSE, README, config example as Resources
# Model binaries are NOT bundled — first-run download only (MODEL-02 packaging constraint)
cp "$REPO_ROOT/LICENSE" "$APP_RESOURCES/LICENSE" 2>/dev/null || true
cp "$REPO_ROOT/README.md" "$APP_RESOURCES/README.md" 2>/dev/null || true
cp "$REPO_ROOT/config.example.json" "$APP_RESOURCES/config.example.json" 2>/dev/null || true
# Safety check: assert no model weight files leaked into the bundle
if find "$APP_BUNDLE" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' \
  -o -name '*.pt' -o -name '*.pth' \) | grep -q .; then
  echo "ERROR: model binary found in release artifact. Model weights must not be bundled." >&2
  find "$APP_BUNDLE" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' \
    -o -name '*.pt' -o -name '*.pth' \) >&2
  exit 1
fi

echo "  .app bundle created: $APP_BUNDLE"

# ── codesign (requires --sign) ─────────────────────────────────────────────────
if $DO_SIGN; then
  if [[ -z "${APPLE_DEVELOPER_ID_APP:-}" ]]; then
    echo "ERROR: APPLE_DEVELOPER_ID_APP is required for --sign"
    exit 1
  fi
  echo "  Signing .app bundle …"
  codesign \
    --force --options runtime \
    --sign "$APPLE_DEVELOPER_ID_APP" \
    --entitlements "$SCRIPT_DIR/entitlements.plist" \
    --deep \
    "$APP_BUNDLE" 2>&1 | tee "$DIST_DIR/codesign.log"
  echo "  Verifying codesign …"
  codesign --verify --verbose=4 "$APP_BUNDLE" 2>&1 | tee -a "$DIST_DIR/codesign.log"
  echo "  codesign: OK"
fi

# ── .dmg ──────────────────────────────────────────────────────────────────────
DMG_PATH="$DIST_DIR/$BINARY_NAME-$VERSION-macos-$ARCH.dmg"
echo "  Creating .dmg: $DMG_PATH"
hdiutil create \
  -volname "tui-translator $VERSION" \
  -srcfolder "$APP_BUNDLE" \
  -ov -format UDZO \
  "$DMG_PATH" 2>&1 | tail -5

if $DO_SIGN; then
  echo "  Signing .dmg …"
  codesign --force --sign "$APPLE_DEVELOPER_ID_APP" "$DMG_PATH" 2>&1 | tee -a "$DIST_DIR/codesign.log"
fi

# ── notarize (requires --notarize) ────────────────────────────────────────────
if $DO_NOTARIZE; then
  if [[ -z "${APPLE_KEYCHAIN_PROFILE:-}" ]]; then
    echo "ERROR: APPLE_KEYCHAIN_PROFILE is required for --notarize"
    echo "  Run: xcrun notarytool store-credentials <profile-name> --apple-id <email> --team-id <TEAM_ID>"
    exit 1
  fi
  echo "  Submitting to Apple Notary Service …"
  xcrun notarytool submit "$DMG_PATH" \
    --keychain-profile "$APPLE_KEYCHAIN_PROFILE" \
    --wait 2>&1 | tee "$DIST_DIR/notarize.log"
  echo "  Stapling …"
  xcrun stapler staple "$DMG_PATH" 2>&1 | tee "$DIST_DIR/stapler.log"
  echo "  Gatekeeper check (spctl) …"
  spctl --assess --type install --verbose "$DMG_PATH" 2>&1 | tee "$DIST_DIR/spctl.log"
  echo "  Notarization: OK"
fi

# ── SHA256SUMS ─────────────────────────────────────────────────────────────────
pushd "$DIST_DIR" >/dev/null
sha256sum "$BINARY_NAME-$VERSION-macos-$ARCH.dmg" > SHA256SUMS 2>/dev/null || \
  shasum -a 256 "$BINARY_NAME-$VERSION-macos-$ARCH.dmg" > SHA256SUMS
echo ""
cat SHA256SUMS
popd >/dev/null

echo ""
echo "=== macOS packaging complete ==="
echo "  dist dir: $DIST_DIR"
ls -lh "$DIST_DIR"
