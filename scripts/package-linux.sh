#!/usr/bin/env bash
# scripts/package-linux.sh
# REL-02 — Linux packaging script for tui-translator (issue #478)
#
# Produces the following artifacts in dist/linux/:
#   tui-translator-<ver>-x86_64-unknown-linux-gnu.tar.gz  (universal tarball)
#   tui-translator-<ver>-amd64.deb                        (Debian / Ubuntu)
#   tui-translator-<ver>-1.x86_64.rpm                     (Fedora / Rocky)
#   tui-translator-<ver>-x86_64.AppImage                  (portable, any distro)
#   SHA256SUMS-<ver>-linux.txt                             (checksums)
#
# Usage:
#   ./scripts/package-linux.sh [--version <ver>] [--target <triple>] [--skip-appimage]
#
# Requirements:
#   - cargo, cargo-deb, cargo-generate-rpm, appimagetool
#   - On Debian/Ubuntu: sudo apt install dpkg-dev rpm appimage-builder
#   - On Fedora:        sudo dnf install dpkg rpm-build appimagetool
#
# The script is intentionally idempotent: re-running it rebuilds artifacts.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# ── Defaults ─────────────────────────────────────────────────────────────────
VERSION="${VERSION:-$(cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys; print(json.load(sys.stdin)['packages'][0]['version'])")}"
TARGET="${TARGET:-x86_64-unknown-linux-gnu}"
SKIP_APPIMAGE="${SKIP_APPIMAGE:-0}"
DIST="${REPO_ROOT}/dist/linux"

# Parse flags
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --target)  TARGET="$2";  shift 2 ;;
    --skip-appimage) SKIP_APPIMAGE=1; shift ;;
    *) echo "Unknown flag: $1" >&2; exit 1 ;;
  esac
done

echo "==> Building tui-translator ${VERSION} for ${TARGET}"
mkdir -p "${DIST}"

# ── 1. Build release binary ───────────────────────────────────────────────────
echo "==> Building release binary"
cargo build --locked --release --target "${TARGET}" --bin tui-translator
BIN="target/${TARGET}/release/tui-translator"
if [[ ! -f "${BIN}" ]]; then
  echo "ERROR: binary not found at ${BIN}" >&2
  exit 1
fi

# ── 2. tar.gz (universal) ─────────────────────────────────────────────────────
TARBALL_NAME="tui-translator-${VERSION}-${TARGET}.tar.gz"
TARBALL_STAGE="${DIST}/tui-translator-${VERSION}"
echo "==> Creating tarball: ${TARBALL_NAME}"
rm -rf "${TARBALL_STAGE}"
mkdir -p "${TARBALL_STAGE}"
cp "${BIN}"                        "${TARBALL_STAGE}/tui-translator"
cp config.example.json             "${TARBALL_STAGE}/" 2>/dev/null || true
cp USAGE.md                        "${TARBALL_STAGE}/" 2>/dev/null || true
cp LICENSE                         "${TARBALL_STAGE}/" 2>/dev/null || true
mkdir -p "${TARBALL_STAGE}/LICENSES"
find assets/licenses -name "*.txt" -exec cp {} "${TARBALL_STAGE}/LICENSES/" \; 2>/dev/null || true
# MODEL-03 packaging constraint: assert no model weight files leaked into the tarball stage
# Model binaries are NOT bundled — first-run download only
if find "${TARBALL_STAGE}" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' -o -name '*.pt' -o -name '*.pth' \) | grep -q .; then
  echo "ERROR: model binary found in release artifact. Model weights must not be bundled." >&2
  find "${TARBALL_STAGE}" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' -o -name '*.pt' -o -name '*.pth' \) >&2
  exit 1
fi
tar -czf "${DIST}/${TARBALL_NAME}" -C "${DIST}" "tui-translator-${VERSION}"
rm -rf "${TARBALL_STAGE}"
echo "    => ${TARBALL_NAME}"

# ── 3. .deb (Debian / Ubuntu) ─────────────────────────────────────────────────
if command -v cargo-deb &>/dev/null; then
  echo "==> Creating .deb package"
  cargo deb --no-build --target "${TARGET}" --output "${DIST}/"
  DEB_NAME=$(ls "${DIST}"/*.deb 2>/dev/null | sort -V | tail -1 | xargs basename)
  echo "    => ${DEB_NAME}"
else
  echo "WARN: cargo-deb not found; skipping .deb. Install: cargo install cargo-deb" >&2
fi

# ── 4. .rpm (Fedora / Rocky) ──────────────────────────────────────────────────
if command -v cargo-generate-rpm &>/dev/null; then
  echo "==> Creating .rpm package"
  cargo generate-rpm --target "${TARGET}" --output "${DIST}/"
  RPM_NAME=$(ls "${DIST}"/*.rpm 2>/dev/null | sort -V | tail -1 | xargs basename)
  echo "    => ${RPM_NAME}"
else
  echo "WARN: cargo-generate-rpm not found; skipping .rpm. Install: cargo install cargo-generate-rpm" >&2
fi

# ── 5. AppImage ───────────────────────────────────────────────────────────────
if [[ "${SKIP_APPIMAGE}" == "0" ]] && command -v appimagetool &>/dev/null; then
  echo "==> Creating AppImage"
  APPDIR="${DIST}/AppDir"
  rm -rf "${APPDIR}"
  mkdir -p "${APPDIR}/usr/bin"
  mkdir -p "${APPDIR}/usr/share/metainfo"
  cp "${BIN}" "${APPDIR}/usr/bin/tui-translator"
  chmod +x "${APPDIR}/usr/bin/tui-translator"

  # Minimal desktop entry (AppImage requires one even for terminal apps)
  cat > "${APPDIR}/tui-translator.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=tui-translator
Exec=tui-translator
Icon=tui-translator
Categories=Utility;
Terminal=true
DESKTOP

  # Minimal 48x48 icon (placeholder — replace with real icon before GA)
  if [[ -f "assets/icons/tui-translator.png" ]]; then
    cp "assets/icons/tui-translator.png" "${APPDIR}/tui-translator.png"
  else
    # Create a 1x1 pixel transparent PNG as placeholder
    printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\nIDATx\x9cc\x00\x01\x00\x00\x05\x00\x01\r\n-\xb4\x00\x00\x00\x00IEND\xaeB`\x82' \
      > "${APPDIR}/tui-translator.png"
  fi

  APPIMAGE_NAME="tui-translator-${VERSION}-x86_64.AppImage"
  ARCH=x86_64 appimagetool "${APPDIR}" "${DIST}/${APPIMAGE_NAME}"
  chmod +x "${DIST}/${APPIMAGE_NAME}"
  rm -rf "${APPDIR}"
  echo "    => ${APPIMAGE_NAME}"
else
  if [[ "${SKIP_APPIMAGE}" != "0" ]]; then
    echo "INFO: AppImage skipped (--skip-appimage)"
  else
    echo "WARN: appimagetool not found; skipping AppImage. Download from https://github.com/AppImage/AppImageKit/releases" >&2
  fi
fi

# ── 6. SHA256SUMS ─────────────────────────────────────────────────────────────
CHECKSUMS_NAME="SHA256SUMS-${VERSION}-linux.txt"
echo "==> Computing checksums: ${CHECKSUMS_NAME}"
(
  cd "${DIST}"
  sha256sum tui-translator-"${VERSION}"-*.tar.gz \
            tui-translator-*.deb \
            tui-translator-*.rpm \
            tui-translator-*.AppImage \
    2>/dev/null | sort > "${CHECKSUMS_NAME}" || true
)
echo "    => ${CHECKSUMS_NAME}"

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "==> Linux packaging complete. Artifacts in ${DIST}/:"
ls -lh "${DIST}/" 2>/dev/null
echo ""
echo "Verify checksums with:"
echo "  cd ${DIST} && sha256sum -c ${CHECKSUMS_NAME}"
