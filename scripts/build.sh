#!/usr/bin/env bash
# Builds newt: Cargo core first, then the Swift shell that links it, then the
# app bundle for release builds.
#
# macOS ships bash 3.2, where "${arr[@]}" on an empty array trips `set -u`;
# the ${arr[@]+...} guard below is what keeps that working.
#
# The ordering is the point — SwiftPM links libnewt_ffi.a, so Cargo must have
# produced it first. This script also decides debug vs release by staging the
# chosen artifact into macos/lib, which is the only path Package.swift knows.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="${1:-debug}"

case "$CONFIG" in
  debug)   CARGO_FLAGS=();            SWIFT_FLAGS=() ;;
  release) CARGO_FLAGS=(--release);   SWIFT_FLAGS=(-c release) ;;
  *) echo "usage: $0 [debug|release]" >&2; exit 2 ;;
esac

echo "==> cargo build ($CONFIG)"
cargo build --manifest-path "$REPO_ROOT/core/Cargo.toml" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}

echo "==> staging core library"
mkdir -p "$REPO_ROOT/macos/lib"
cp "$REPO_ROOT/core/target/$CONFIG/libnewt_ffi.a" "$REPO_ROOT/macos/lib/libnewt_ffi.a"

echo "==> swift build ($CONFIG)"
swift build --package-path "$REPO_ROOT/macos" ${SWIFT_FLAGS[@]+"${SWIFT_FLAGS[@]}"}

BINARY="$REPO_ROOT/macos/.build/$CONFIG/NewtApp"

if [ "$CONFIG" != "release" ]; then
  echo "==> built $BINARY"
  exit 0
fi

APP="$REPO_ROOT/newt.app"
RESOURCES="$REPO_ROOT/macos/Resources"

echo "==> bundling $APP"
# The icon is generated rather than checked in; regenerate only when missing so
# a release build does not depend on a stale artifact or rebuild it needlessly.
if [ ! -f "$RESOURCES/newt.icns" ]; then
  swift "$REPO_ROOT/scripts/make-icon.swift" "$RESOURCES/newt.icns"
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# The bundle executable is named for the product, not the SwiftPM target.
cp "$BINARY" "$APP/Contents/MacOS/newt"
cp "$RESOURCES/Info.plist" "$APP/Contents/Info.plist"
cp "$RESOURCES/newt.icns" "$APP/Contents/Resources/newt.icns"

echo "==> signing (ad-hoc)"
# Absolute path on purpose: package managers (conda among them) put their own
# `codesign` shim earlier on PATH, and it does not take Apple's arguments.
#
# Ad-hoc is enough to run locally and needs no Apple Developer account. There is
# no nested code, so --deep (deprecated anyway) is unnecessary.
/usr/bin/codesign --force --sign - "$APP"
/usr/bin/codesign --verify --strict "$APP"

echo "==> built $APP"
echo "    run:     open $APP"
echo "    install: cp -R $APP /Applications/"
