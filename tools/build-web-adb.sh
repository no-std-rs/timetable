#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
RUSTFLAGS="--cfg=web_sys_unstable_apis" cargo +1.95.0 build --manifest-path android/provisioner/Cargo.toml --locked --release --target wasm32-unknown-unknown
mkdir -p web/dist/android-adb
wasm-bindgen --target web --out-dir web/dist/android-adb android/provisioner/target/wasm32-unknown-unknown/release/timetable_web_adb.wasm
