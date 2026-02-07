# nodpi

## nodpi ui for multi platforms

### how it build
    - build apps/server_rust/ for many platforms
        - aarch64-linux-android
        - armv7-linux-androideabi
        - i686-linux-android 
        - aarch64-apple-darwin
    - place build script to related app
    - 

#### description
    - server on rust
        - @apps/server_rust
    - use dioxus (rust) as framework for ui clients
        - @apps/ui_android_dioxusu
        - @apps/ui_desktop_dioxus
    - pure android version via kotlin
        - @apps/ui_android_kotlin

#### ui plan features
    - switch between fragment method in ui
    - edit default blacklist as text file / @blacklist.txt as default file
    - edit port, default is 8881
    - edit host, default is 0.0.0.0

#### how to build server for desktop
```
cargo build --manifest-path apps/server_rust/Cargo.toml
```

#### how to build server for android
```
cargo install cargo-ndk
cargo ndk --target aarch64-linux-android --target armv7-linux-androideabi --target i686-linux-android build
```
