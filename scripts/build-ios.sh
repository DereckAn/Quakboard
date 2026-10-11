#!/bin/bash
# Builds the sync core for iOS and packages it as the local Swift package
# ios/QuakboardSync: Swift bindings first, then the XCFramework, which needs
# the generated header.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/crates/quakboard-sync-ffi"
PACKAGE="$ROOT/ios/QuakboardSync"
# Shares the cache with the desktop and CI builds.
export CARGO_TARGET_DIR="$ROOT/src-tauri/target"
# Matches the app's minimum, so the linker doesn't see a newer iOS.
export IPHONEOS_DEPLOYMENT_TARGET=16.0

LIB=libquakboard_sync_ffi.a
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)

cd "$CRATE"
for target in "${TARGETS[@]}"; do
    cargo build --release --target "$target"
done

GEN="$CARGO_TARGET_DIR/ios-bindings"
rm -rf "$GEN"
cargo run --release --features bindgen --bin uniffi-bindgen -- generate \
    --library "$CARGO_TARGET_DIR/aarch64-apple-ios/release/libquakboard_sync_ffi.dylib" \
    --language swift --out-dir "$GEN"

# The XCFramework only picks up the module map under this exact name.
HEADERS="$GEN/headers"
mkdir -p "$HEADERS"
cp "$GEN/quakboard_sync_ffiFFI.h" "$HEADERS/"
cp "$GEN/quakboard_sync_ffiFFI.modulemap" "$HEADERS/module.modulemap"

# One simulator slice holds both Apple Silicon and Intel Macs.
SIM="$CARGO_TARGET_DIR/ios-simulator"
mkdir -p "$SIM"
lipo -create \
    "$CARGO_TARGET_DIR/aarch64-apple-ios-sim/release/$LIB" \
    "$CARGO_TARGET_DIR/x86_64-apple-ios/release/$LIB" \
    -output "$SIM/$LIB"

rm -rf "$PACKAGE/QuakboardSyncFFI.xcframework"
xcodebuild -create-xcframework \
    -library "$CARGO_TARGET_DIR/aarch64-apple-ios/release/$LIB" -headers "$HEADERS" \
    -library "$SIM/$LIB" -headers "$HEADERS" \
    -output "$PACKAGE/QuakboardSyncFFI.xcframework"

mkdir -p "$PACKAGE/Sources/QuakboardSync"
cp "$GEN/quakboard_sync_ffi.swift" "$PACKAGE/Sources/QuakboardSync/"

echo "Built $PACKAGE"
