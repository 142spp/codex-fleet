#!/usr/bin/env python3
"""Render the real synthetic demo layout as a dependency-free README SVG."""
import html
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
width, height = 140, 34
with tempfile.TemporaryDirectory() as directory:
    snapshot = Path(directory) / "demo.txt"
    subprocess.run([str(root / "target/debug/codex-fleet"), "--demo", "--width", str(width),
                    "--height", str(height), "--snapshot", str(snapshot)], check=True)
    lines = snapshot.read_text().splitlines()

svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width * 9 + 40}" height="{height * 20 + 40}" role="img" aria-label="Codex Fleet synthetic demo">',
       '<rect width="100%" height="100%" rx="10" fill="#19191c"/>',
       '<g font-family="DejaVu Sans Mono, monospace" font-size="14" xml:space="preserve">']
for row, line in enumerate(lines):
    y = 20 + row * 20
    selected = line.startswith("›")
    if selected:
        svg.append(f'<rect x="18" y="{y - 2}" width="790" height="20" fill="#285e98"/>')
    color = "#ffffff" if selected or row < 2 else "#cecbd8"
    svg.append(f'<text x="20" y="{y + 14}" fill="{color}">{html.escape(line.rstrip())}</text>')
svg.extend(["</g>", "</svg>"])
output = root / "docs/assets/demo.svg"
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text("\n".join(svg) + "\n")
print(output.relative_to(root))
