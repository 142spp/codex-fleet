#!/usr/bin/env python3
"""Export the real TUI cell buffer; do not approximate its layout or colors."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
output = root / "docs/assets/demo.svg"
output.parent.mkdir(parents=True, exist_ok=True)
subprocess.run([str(root / "target/debug/codex-fleet"), "--demo", "--width", "140",
                "--height", "34", "--snapshot", str(output)], check=True)
print(output.relative_to(root))
