#!/bin/sh
set -eu
repo_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
output_dir=${1:-"$repo_dir/dist/firmware"}
mkdir -p "$output_dir"
output_dir=$(CDPATH='' cd -- "$output_dir" && pwd)
cd "$repo_dir/firmware"
# Public firmware never reads compile-time credentials. Settings arrive over USB.
cargo build --locked --release --features firmware
target_dir=${CARGO_TARGET_DIR:-target}
elf="$target_dir/riscv32imc-unknown-none-elf/release/timetable-display"
python3 "$repo_dir/tools/check-device-ram.py" "$elf" "$output_dir/RAM.json"
python3 "$repo_dir/tools/check-device-stack.py" "$elf" "$output_dir/STACK.json"
espflash save-image --skip-update-check --chip esp32c3 --merge --skip-padding \
    --flash-mode dio --flash-freq 80mhz --flash-size 4mb --partition-table partitions.csv \
    "$elf" "$output_dir/timetable-display.bin"
python3 "$repo_dir/tools/package-device.py" "$output_dir"
