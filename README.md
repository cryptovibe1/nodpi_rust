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
