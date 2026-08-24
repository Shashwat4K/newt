#!/usr/bin/env bash
# Builds newt: Cargo core first, then the Swift shell that links it.
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

echo "==> built $REPO_ROOT/macos/.build/$CONFIG/NewtApp"
