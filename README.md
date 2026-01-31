### Rust port of https://github.com/GVCoder09/NoDPI

### run
```
cargo run -- --host 0.0.0.0 --port 8881 --blacklist blacklist.txt
```

### release build
```
cargo build --release
./target/release/nodpi_rust --host 0.0.0.0 --port 8881 --blacklist blacklist.txt
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
