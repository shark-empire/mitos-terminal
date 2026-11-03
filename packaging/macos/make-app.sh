#!/usr/bin/env bash
#
# Builds "MITOS Terminal.app" from the release binary, and optionally installs it and/or wraps it
# in a .dmg disk image. Run it on a Mac, from anywhere inside the repository:
#
#   packaging/macos/make-app.sh --build --install      # compile, make the app, copy to Applications
#   packaging/macos/make-app.sh --build --dmg          # compile, make the app and a .dmg
#   packaging/macos/make-app.sh --binary PATH --dmg    # use a binary you already built (e.g. in CI)
#
# Options:
#   --build        run `cargo build --release --no-default-features` first
#   --binary PATH  the mitos-terminal executable to put in the app
#                  (default: target/release/mitos-terminal)
#   --out DIR      where to put the app / dmg (default: dist/macos)
#   --dmg          also create MITOS-Terminal-<version>-<arch>.dmg (drag-to-Applications image)
#   --install      copy the app to /Applications (or ~/Applications if that is not writable)
#   --no-sign      skip the ad-hoc code signature
#   -h, --help     this text
#
# Environment: MITOS_BUNDLE_ID overrides the bundle identifier (default org.mitos.terminal).
#
# The app is only ad-hoc signed (no Apple Developer ID), so a copy that was downloaded needs the
# one-time "Open Anyway" step described in docs/MACOS.md; a copy you build and install yourself
# does not.
#
# macOS ships bash 3.2, so this script sticks to features that old bash has.

set -euo pipefail

APP_NAME="MITOS Terminal"
EXE_NAME="mitos-terminal"
BUNDLE_ID="${MITOS_BUNDLE_ID:-org.mitos.terminal}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

build=0
dmg=0
install=0
sign=1
binary=""
out="$root/dist/macos"

usage() {
  # Print the header comment above (everything up to `set -euo pipefail`).
  sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --build) build=1 ;;
    --binary) binary="${2:?--binary needs a path}"; shift ;;
    --out) out="${2:?--out needs a directory}"; shift ;;
    --dmg) dmg=1 ;;
    --install) install=1 ;;
    --no-sign) sign=0 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [ "$(uname -s)" != "Darwin" ] && [ -z "${MITOS_PACKAGING_ALLOW_NON_MAC:-}" ]; then
  echo "make-app.sh builds a macOS app and has to be run on a Mac." >&2
  exit 1
fi

case "$BUNDLE_ID" in
  *[!A-Za-z0-9.-]*|"") echo "MITOS_BUNDLE_ID may only contain letters, digits, '.' and '-'." >&2; exit 2 ;;
esac

if [ "$build" = 1 ]; then
  echo "==> cargo build --release --no-default-features"
  ( cd "$root" && cargo build --release --no-default-features )
fi

if [ -z "$binary" ]; then
  binary="$root/target/release/$EXE_NAME"
fi
if [ ! -f "$binary" ]; then
  echo "Binary not found: $binary" >&2
  echo "Build it first (cargo build --release --no-default-features) or pass --build." >&2
  exit 1
fi

version="$(sed -n 's/^version *= *"\(.*\)".*/\1/p' "$root/Cargo.toml" | head -n 1)"
[ -n "$version" ] || version="0.0.0"
arch="$(lipo -archs "$binary" 2>/dev/null | tr ' ' '-' || true)"
[ -n "$arch" ] || arch="$(uname -m)"

# The oldest macOS the binary itself was built for (CI sets MACOSX_DEPLOYMENT_TARGET=11.0; a build
# on your own Mac targets that Mac's system). The app's Info.plist must not claim more than that,
# or Finder would refuse to open an app that would actually run.
min_os=""
if command -v otool >/dev/null 2>&1; then
  min_os="$(otool -l "$binary" 2>/dev/null | awk '
    /cmd LC_BUILD_VERSION/ { build = 1 }
    /cmd LC_VERSION_MIN_MACOSX/ { old = 1 }
    build && $1 == "minos" { print $2; exit }
    old && $1 == "version" { print $2; exit }
  ' || true)"
fi
[ -n "$min_os" ] || min_os="10.13"

app="$out/$APP_NAME.app"
echo "==> assembling $app (version $version, $arch, macOS $min_os or newer)"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$binary" "$app/Contents/MacOS/$EXE_NAME"
chmod 755 "$app/Contents/MacOS/$EXE_NAME"
cp "$here/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
if [ -f "$root/.config/mitos/terminal.toml" ]; then
  cp "$root/.config/mitos/terminal.toml" "$app/Contents/Resources/terminal.toml.example"
fi

sed -e "s/@EXE_NAME@/$EXE_NAME/g" \
    -e "s/@APP_NAME@/$APP_NAME/g" \
    -e "s/@BUNDLE_ID@/$BUNDLE_ID/g" \
    -e "s/@VERSION@/$version/g" \
    -e "s/@MIN_OS@/$min_os/g" \
    "$here/Info.plist.in" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$app/Contents/Info.plist" >/dev/null
fi

if [ "$sign" = 1 ] && command -v codesign >/dev/null 2>&1; then
  echo "==> ad-hoc signing"
  codesign --force --deep --sign - "$app"
fi

if [ "$install" = 1 ]; then
  dest="/Applications"
  if [ ! -w "$dest" ]; then
    dest="$HOME/Applications"
    mkdir -p "$dest"
  fi
  echo "==> installing to $dest"
  rm -rf "$dest/$APP_NAME.app"
  ditto "$app" "$dest/$APP_NAME.app"
  # A copy built on this Mac is not quarantined, but be sure.
  xattr -dr com.apple.quarantine "$dest/$APP_NAME.app" 2>/dev/null || true
  installed="$dest/$APP_NAME.app"
fi

if [ "$dmg" = 1 ]; then
  stage="$(mktemp -d "${TMPDIR:-/tmp}/mitos-dmg.XXXXXX")"
  ditto "$app" "$stage/$APP_NAME.app"
  ln -s /Applications "$stage/Applications"
  dmg_path="$out/MITOS-Terminal-$version-$arch.dmg"
  rm -f "$dmg_path"
  echo "==> creating $dmg_path"
  hdiutil create -volname "$APP_NAME" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg_path" >/dev/null
  rm -rf "$stage"
fi

echo
echo "Done."
echo "  App:        $app"
[ -z "${installed:-}" ] || echo "  Installed:  $installed"
[ "$dmg" != 1 ] || echo "  Disk image: $dmg_path"
if [ -z "${installed:-}" ]; then
  echo
  echo "To install: drag \"$APP_NAME.app\" into your Applications folder"
  echo "(or run this script again with --install)."
fi
