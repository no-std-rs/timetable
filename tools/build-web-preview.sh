#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
# Fixed private memory: single-threaded ABI, no allocator, no host imports.
RUSTFLAGS="-C link-arg=--initial-memory=2097152 -C link-arg=--max-memory=2097152"   cargo +1.95.0 rustc -p timetable-web-preview --lib --crate-type cdylib --release --locked --target wasm32-unknown-unknown
mkdir -p web/dist
cp target/wasm32-unknown-unknown/release/timetable_web_preview.wasm web/dist/device-preview.wasm
