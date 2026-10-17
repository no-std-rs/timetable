"""Check complete async task storage for a target ABI, without running its code."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--target", default="riscv32imac-unknown-none-elf")
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
env = os.environ.copy()
target_dir = Path(env.get("CARGO_TARGET_DIR", str(root / "target"))).resolve()
env["CARGO_TARGET_DIR"] = str(target_dir)
subprocess.run([
    "cargo", "rustc", "--manifest-path", str(root / "checks/ram/Cargo.toml"),
    "--locked", "--offline", "--release", "--target", args.target, "--", "--emit=llvm-ir",
], check=True, env=env)
ir_file = max((target_dir / args.target / "release/deps").glob("timetable_ram_probe-*.ll"), key=lambda p: p.stat().st_mtime)
ir = ir_file.read_text()

def constant(name):
    match = re.search(r"^define[^\n]*" + name + r"[^\n]*\n(.*?)^}", ir, re.M | re.S)
    if not match:
        raise RuntimeError(f"missing {name} in {ir_file}")
    result = re.search(r"ret i(?:32|64) (\d+)", match[1])
    if not result:
        raise RuntimeError(f"compiler did not fold {name} to a constant")
    return int(result[1])

reserve = constant("reserves_bytes")
rows = []
for capacity in [1024, 2048, 4096]:
    task = constant(f"task_state_bytes_{capacity}")
    total = task + reserve
    rows.append(dict(trips=capacity, task_bytes=task, reserves_bytes=reserve, total_bytes=total, fits_400000=total <= 400_000))
print(json.dumps(dict(target=args.target, profiles=rows), indent=2))
if not rows[1]["fits_400000"]:
    raise SystemExit("2048-trip embedded profile exceeds the 400,000-byte budget")
