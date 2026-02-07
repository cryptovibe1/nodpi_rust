#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_JNILIBS="$ROOT_DIR/apps/ui_android_kotlin/app/src/main/jniLibs"

echo "Building nodpi_server for Android..."
cargo ndk \
  --manifest-path apps/server_rust/Cargo.toml \
  --target aarch64-linux-android \
  --target armv7-linux-androideabi \
  --target i686-linux-android \
  build

echo "Copying binaries to jniLibs..."
mkdir -p "$APP_JNILIBS/arm64-v8a" "$APP_JNILIBS/armeabi-v7a" "$APP_JNILIBS/x86"

cp "$ROOT_DIR/target/aarch64-linux-android/debug/nodpi_server" \
  "$APP_JNILIBS/arm64-v8a/libnodpi_server.so"

cp "$ROOT_DIR/target/armv7-linux-androideabi/debug/nodpi_server" \
  "$APP_JNILIBS/armeabi-v7a/libnodpi_server.so"

cp "$ROOT_DIR/target/i686-linux-android/debug/nodpi_server" \
  "$APP_JNILIBS/x86/libnodpi_server.so"

echo "Done."
