#!/bin/sh
set -eu
repo_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
output_dir=${1:-"$repo_dir/dist/firmware"}
mkdir -p "$output_dir"
output_dir=$(CDPATH='' cd -- "$output_dir" && pwd)
board=${2:-guition-c3}
# C6 ROM uses a different flash-frequency encoding (esptool esp32c6.py).
# Use the conservative supported 40 MHz header; C3 retains its 80 MHz image.
case "$board" in
    guition-c3) chip=esp32c3; target=riscv32imc-unknown-none-elf; feature=board-guition; flash_size=4mb; flash_freq=80mhz ;;
    waveshare-c6) chip=esp32c6; target=riscv32imac-unknown-none-elf; feature=board-waveshare-c6; flash_size=8mb; flash_freq=40mhz ;;
    waveshare-c6-landscape) chip=esp32c6; target=riscv32imac-unknown-none-elf; feature=c6-landscape; flash_size=8mb; flash_freq=40mhz ;;
    *) echo "Unsupported display: $board" >&2; exit 1 ;;
esac
cd "$repo_dir/firmware"
# Public firmware never reads compile-time credentials. Settings arrive over USB.
cargo build --locked --release --no-default-features --features "firmware,$feature" --target "$target"
target_dir=${CARGO_TARGET_DIR:-target}
elf="$target_dir/$target/release/timetable-display"
python3 "$repo_dir/tools/check-device-ram.py" "$elf" "$output_dir/RAM.json" "${board%-landscape}"
python3 "$repo_dir/tools/check-device-stack.py" "$elf" "$output_dir/STACK.json"
espflash save-image --skip-update-check --chip "$chip" --merge --skip-padding \
    --flash-mode dio --flash-freq "$flash_freq" --flash-size "$flash_size" --partition-table partitions.csv \
    "$elf" "$output_dir/timetable-display.bin"
python3 "$repo_dir/tools/package-device.py" "$output_dir" "$board"
