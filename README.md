### Rust port of https://github.com/GVCoder09/NoDPI

### run rust server
```
cargo run --manifest-path apps/server_rust/Cargo.toml -- --host 0.0.0.0 --port 8881 --blacklist blacklist.txt
```

### Как использовать

1. Создать пользователя:

```
cargo run --manifest-path apps/server_rust/Cargo.toml -- --add-user alice --add-pass secret --users-file users.txt
```

2. Запустить прокси с авторизацией из файла:
```
cargo run --manifest-path apps/server_rust/Cargo.toml -- --users-file users.txt
```

### build servers for ui android kotlin
```
bash scripts/android-build-server.sh
```

### build android apk
```
cd apps/ui_android_kotlin
gradle assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

### build servers for android
```
cargo install cargo-ndk
cargo ndk --manifest-path apps/server_rust/Cargo.toml --target aarch64-linux-android --target armv7-linux-androideabi --target i686-linux-android build
```

### setup java and android sdk
```
brew install gradle
brew install openjdk
sudo ln -sfn $(brew --prefix)/opt/openjdk/libexec/openjdk.jdk /Library/Java/JavaVirtualMachines/openjdk.jdk
java -version
brew install --cask android-ndk
brew install --cask android-commandlinetools
brew install android-platform-tools
sdkmanager --install platform-tools
sdkmanager "emulator"
export ANDROID_NDK_HOME="/opt/homebrew/share/android-ndk"
export ANDROID_HOME="/opt/homebrew/share/android-commandlinetools"
export PATH=$PATH:$ANDROID_HOME/emulator
```

### Как использовать

1. Создать пользователя:

```
cargo run -- --add-user alice --add-pass secret --users-file users.txt
```

2. Запустить прокси с авторизацией из файла:

```
cargo run -- --users-file users.txt
```
