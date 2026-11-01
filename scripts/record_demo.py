#!/usr/bin/env python3
"""Record the actual demo TUI through an isolated tmux server; encode with ImageMagick.

Requires Python 3.11+, tmux, ImageMagick 7 and DejaVu Sans Mono.
No Python packages, Codex credentials or network connections are needed.
"""
import argparse
from contextlib import nullcontext
from html import escape
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import unicodedata

ROOT = Path(__file__).resolve().parents[1]
COLS, ROWS = 140, 34
CELL_W, CELL_H = 9, 20
PAD, TOP = 20, 76
WIDTH, HEIGHT = COLS * CELL_W + PAD * 2, ROWS * CELL_H + TOP + PAD
BACKGROUND, FOREGROUND = "#19191c", "#cecbd8"
# Match the reference palette in src/snapshot.rs.
PALETTE = ["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc",
           "#11a8cd", "#e5e5e5", "#666666", "#f14c4c", "#23d18b", "#f5f543",
           "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]
SGR = re.compile(r"\x1b\[([0-9;]*)m")


def indexed_color(index):
    if index < 16:
        return PALETTE[index]
    if index < 232:
        value = index - 16
        levels = [0, 95, 135, 175, 215, 255]
        return "#{:02x}{:02x}{:02x}".format(
            levels[value // 36], levels[value // 6 % 6], levels[value % 6])
    return "#{0:02x}{0:02x}{0:02x}".format(8 + 10 * (index - 232))


def frame_svg(capture, columns, title, key, number, total):
    """Render captured SGR attributes and cell positions, not a recreated UI."""
    output = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}">',
              f'<rect width="100%" height="100%" rx="12" fill="{BACKGROUND}"/>',
              '<g font-family="DejaVu Sans Mono, monospace">',
              '<text x="20" y="28" fill="#78c8a0" font-size="12" font-weight="bold">CODEX FLEET / QUICK TOUR</text>',
              f'<text x="20" y="53" fill="#ffffff" font-size="17">{escape(title)}</text>',
              f'<text x="{WIDTH - 20}" y="28" text-anchor="end" fill="#9999a5" font-size="12">{number:02d} / {total:02d} · SYNTHETIC DEMO</text>',
              f'<text x="{WIDTH - 20}" y="53" text-anchor="end" fill="#8ab4f8" font-size="14">{escape(key)}</text>',
              f'<path d="M20 65H{WIDTH - 20}" stroke="#35353d"/>',
              '<g font-size="14" xml:space="preserve">']
    style = dict(fg=FOREGROUND, bg=BACKGROUND, bold=False, dim=False,
                 italic=False, underline=False, reverse=False)
    defaults = style.copy()
    origin = PAD + (COLS - columns) * CELL_W // 2
    for row, line in enumerate(capture.splitlines()):
        if row >= ROWS:
            break
        column = 0
        # Keep style between lines: tmux emits deltas, not independent rows.
        for token in re.split(r"(\x1b\[[0-9;]*m)", line):
            if match := SGR.fullmatch(token):
                codes = [int(value or 0) for value in match[1].split(";")]
                i = 0
                while i < len(codes):
                    code = codes[i]
                    if code == 0:
                        style = defaults.copy()
                    elif code in (1, 2, 3, 4, 7):
                        style[{1: "bold", 2: "dim", 3: "italic", 4: "underline", 7: "reverse"}[code]] = True
                    elif code == 22:
                        style.update(bold=False, dim=False)
                    elif code in (23, 24, 27):
                        style[{23: "italic", 24: "underline", 27: "reverse"}[code]] = False
                    elif code in (39, 49):
                        field = "fg" if code == 39 else "bg"
                        style[field] = defaults[field]
                    elif 30 <= code <= 37 or 90 <= code <= 97:
                        style["fg"] = PALETTE[code - (30 if code < 90 else 82)]
                    elif 40 <= code <= 47 or 100 <= code <= 107:
                        style["bg"] = PALETTE[code - (40 if code < 100 else 92)]
                    elif code in (38, 48):
                        field = "fg" if code == 38 else "bg"
                        mode = codes[i + 1]
                        if mode == 5:
                            style[field] = indexed_color(codes[i + 2]); i += 2
                        elif mode == 2:
                            style[field] = "#{:02x}{:02x}{:02x}".format(*codes[i + 2:i + 5]); i += 4
                        else:
                            raise ValueError(f"Unsupported color sequence: {token!r}")
                    else:
                        raise ValueError(f"Unsupported SGR attribute: {code}")
                    i += 1
                continue
            if "\x1b" in token:
                raise ValueError(f"Unexpected terminal control: {token!r}")
            for char in token:
                if unicodedata.combining(char):
                    raise ValueError("Combining characters need explicit cell handling")
                cells = 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
                x, y = origin + column * CELL_W, TOP + row * CELL_H
                fg, bg = style["fg"], style["bg"]
                if style["reverse"]:
                    fg, bg = bg, fg
                if bg != BACKGROUND:
                    output.append(f'<rect x="{x}" y="{y}" width="{cells * CELL_W}" height="{CELL_H}" fill="{bg}"/>')
                if not char.isspace():
                    weight = "bold" if style["bold"] else "normal"
                    italic = "italic" if style["italic"] else "normal"
                    underline = "underline" if style["underline"] else "none"
                    opacity = "0.65" if style["dim"] else "1"
                    output.append(f'<text x="{x}" y="{y + 15}" fill="{fg}" font-weight="{weight}" font-style="{italic}" text-decoration="{underline}" opacity="{opacity}">{escape(char)}</text>')
                column += cells
    output.append('</g></g></svg>')
    return "\n".join(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/assets/demo.gif")
    parser.add_argument("--frames-dir", type=Path, help="Keep SVG/PNG frames for review")
    args = parser.parse_args()
    binary = ROOT / "target/debug/codex-fleet"
    if not binary.is_file():
        parser.error("Build the application first: cargo build --locked")
    for command in ("tmux", "magick"):
        if not shutil.which(command):
            parser.error(f"Required command is missing: {command}")

    # Each step operates the running demo with real keys. Durations are playback
    # pauses for readability; there is no simulated server or invented UI state.
    steps = [
        ("Your devices, grouped by project", "Device → folder → session", [], COLS, 180),
        ("Move through sessions and read the preview", "j / k · navigate", ["j"], COLS, 110),
        ("Inspect work on another device", "j · next session", ["j"], COLS, 110),
        ("Find tasks waiting for you", "Tab · status filter", ["Tab"], COLS, 160),
        ("See what is running", "Tab · Working", ["Tab"], COLS, 140),
        ("Search across the entire fleet", "/ · search", ["BTab", "BTab", "/", "a", "p", "i"], COLS, 160),
        ("Group by project across devices", "g · grouping", ["Escape", "Escape", "g"], COLS, 140),
        ("Switch to a flat session list", "g · grouping", ["g"], COLS, 120),
        ("Focus on one device", "c · device filter", ["g", "c"], COLS, 140),
        ("Keep the shortcuts close", "? · keyboard guide", ["c", "c", "c", "?"], COLS, 190),
        ("A compact view for smaller terminals", "95 columns", ["Escape"], 95, 180),
        ("More room, more context", "140 columns", [], COLS, 180),
    ]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.frames_dir:
        args.frames_dir.mkdir(parents=True, exist_ok=True)
    workspace = nullcontext(str(args.frames_dir)) if args.frames_dir else tempfile.TemporaryDirectory(prefix="fleet-frames-")
    with workspace as directory, tempfile.TemporaryDirectory(prefix="fleet-record-") as session:
        frames = Path(directory)
        tmux_args = ["tmux", "-S", str(Path(session) / "tmux.sock")]
        env = dict(os.environ)
        env.pop("NO_COLOR", None)
        env["TERM"] = "xterm-256color"

        def tmux(*command, check=True):
            return subprocess.run(tmux_args + list(command), env=env, check=check,
                                  capture_output=True, text=True).stdout

        manifest = []
        try:
            tmux("-f", "/dev/null", "new-session", "-d", "-s", "demo", "-x", str(COLS),
                 "-y", str(ROWS), str(binary), "--demo")
            time.sleep(0.4)
            previous_columns = COLS
            for i, (title, key, keys, columns, delay) in enumerate(steps, 1):
                for pressed in keys:
                    tmux("send-keys", "-t", "demo", pressed)
                    # Give standalone Escape time to decode before the next
                    # key, which could otherwise be interpreted as Alt+key.
                    time.sleep(0.15 if pressed == "Escape" else 0.06)
                if columns != previous_columns:
                    tmux("resize-window", "-t", "demo", "-x", str(columns), "-y", str(ROWS))
                time.sleep(0.4)  # Includes the 200 ms preview debounce.
                capture = tmux("capture-pane", "-p", "-e", "-t", "demo")
                plain = SGR.sub("", capture)
                assert "Codex Fleet" in plain, "Demo exited unexpectedly"
                if columns >= 110 and "?" not in keys:
                    assert "Task details" in plain, "Wide-screen preview is missing"
                if i == 6:
                    assert "/api" in plain and "All 1 " in plain, "Search did not filter the list"
                if i >= 7:
                    assert "/api" not in plain, "Search was not cleared before grouping"
                if i == 10:
                    assert "Task shortcuts" in plain, "Shortcut guide did not open"
                if columns < 110:
                    assert "Task details" not in plain, "Compact view retained the side panel"
                path = frames / f"{i:02d}"
                path.with_suffix(".ansi").write_text(capture)
                path.with_suffix(".svg").write_text(frame_svg(capture, columns, title, key, i, len(steps)))
                subprocess.run(["magick", "-background", BACKGROUND, str(path.with_suffix(".svg")),
                                "-alpha", "remove", str(path.with_suffix(".png"))], check=True)
                manifest.append({"frame": i, "title": title, "keys": keys, "columns": columns, "delay_cs": delay})
                previous_columns = columns
                print(f"{i:02d}/{len(steps)} {title}", flush=True)
        finally:
            tmux("kill-server", check=False)
        (frames / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        command = ["magick"]
        for frame in manifest:
            command += ["-delay", str(frame["delay_cs"]), str(frames / f'{frame["frame"]:02d}.png')]
        command += ["-loop", "0", "-layers", "Optimize", str(args.output)]
        subprocess.run(command, check=True)
    print(f"Saved {args.output} ({args.output.stat().st_size:,} bytes)")


if __name__ == "__main__":
    main()
