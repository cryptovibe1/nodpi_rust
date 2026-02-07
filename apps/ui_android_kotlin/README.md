# NoDPI Android (Kotlin)

This app controls the `nodpi_server` binary on Android and edits the same config
format as the Rust UI. It copies the server binary from assets on first launch.

## Build
```
cd apps/ui_android_kotlin
gradle assembleDebug
```

## Server binary assets
Place built binaries under:
- `app/src/main/assets/servers/arm64-v8a/nodpi_server`
- `app/src/main/assets/servers/armeabi-v7a/nodpi_server`
- `app/src/main/assets/servers/x86/nodpi_server`
- `app/src/main/assets/servers/x86_64/nodpi_server`

If the device blocks execution from app data, use jniLibs instead:
- `app/src/main/jniLibs/arm64-v8a/libnodpi_server.so` (or `nodpi_server.so`)
- `app/src/main/jniLibs/armeabi-v7a/libnodpi_server.so` (or `nodpi_server.so`)
- `app/src/main/jniLibs/x86/libnodpi_server.so` (or `nodpi_server.so`)
- `app/src/main/jniLibs/x86_64/libnodpi_server.so` (or `nodpi_server.so`)

Build binaries with:
```
cargo install cargo-ndk
cargo ndk --target aarch64-linux-android --target armv7-linux-androideabi --target i686-linux-android build
```
