cd M:\0-CRDD\Rust\dubDSR
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
cargo build --release