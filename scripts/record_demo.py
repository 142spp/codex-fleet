#!/usr/bin/env python3
"""Record the actual demo TUI through an isolated tmux server; encode a continuous keyboard screencast.

Requires Python 3.11+, tmux, ImageMagick 7, FFmpeg and DejaVu Sans Mono.
No Python packages, Codex credentials or network connections are needed.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
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
PAD, TOP = 20, 20
WIDTH, HEIGHT = COLS * CELL_W + PAD * 2, ROWS * CELL_H + TOP + PAD
FPS, DURATION = 20, 24
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


def frame_svg(capture):
    """Preserve captured terminal cells without adding presentation overlays."""
    output = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}">',
              f'<rect width="100%" height="100%" rx="12" fill="{BACKGROUND}"/>',
              '<g font-family="DejaVu Sans Mono, monospace">',
              '<g font-size="14" xml:space="preserve">']
    style = dict(fg=FOREGROUND, bg=BACKGROUND, bold=False, dim=False,
                 italic=False, underline=False, reverse=False)
    defaults = style.copy()
    origin = PAD
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
    output.append('</g>')
    output.append('</g></svg>')
    return "\n".join(output)


# One uninterrupted task: browse, filter, type a search, read, and return.
# Uneven pauses include time to read and inspect the asynchronously loaded panel.
EVENTS = [
    (1.05, 'Down', '↓', 'Next session'),
    (1.42, 'Down', '↓', 'Next session'),
    (1.65, 'Down', '↓', 'Next session'),
    (2.90, 'Up', '↑', 'Previous session'),
    (3.80, 'Tab', 'Tab', 'Filter: Needs you'),
    (5.30, 'BTab', 'Shift+Tab', 'Filter: All'),
    (6.25, '/', '/', 'Search sessions'),
    (6.85, 'c', 'c', 'Search: c'),
    (7.02, 'a', 'a', 'Search: ca'),
    (7.14, 'c', 'c', 'Search: cac'),
    (7.40, 'h', 'h', 'Search: cach'),
    (7.54, 'e', 'e', 'Search: cache'),
    (8.65, 'Enter', 'Enter', 'Keep search results'),
    (9.15, 'd', 'd', 'Open conversation'),
    (10.80, 'Escape', 'Esc', 'Close conversation'),
    (11.30, 'Escape', 'Esc', 'Clear search'),
    (12.25, 'g', 'g', 'Group by project'),
    (13.05, 'Down', '↓', 'Next session'),
    (13.23, 'Down', '↓', 'Next session'),
    (14.40, 'g', 'g', 'Flat session list'),
    (15.00, 'g', 'g', 'Group by device'),
    (16.00, '?', '?', 'Open keyboard guide'),
    (17.90, 'Escape', 'Esc', 'Close keyboard guide'),
    (18.60, 'Home', 'Home', 'First session'),
    (19.20, 'Down', '↓', 'Next session'),
    (19.40, 'Down', '↓', 'Next session'),
    (19.80, 'Up', '↑', 'Previous session'),
    (20.15, 'Home', 'Home', 'First session'),
]


def validate_recording(recording):
    def pane(at):
        frame = min(recording, key=lambda item: abs(item['time'] - at))
        return SGR.sub('', frame['capture'])

    assert 'All 6 ' in pane(0.5)
    assert 'Needs you 1' in pane(4.5) and 'Run integration tests' in pane(4.5)
    assert '/ca' in pane(7.12), 'Intermediate typing was not captured'
    assert '/cache' in pane(8.2) and 'All 1 ' in pane(8.2)
    assert 'Recent conversation' in pane(9.8)
    assert '/cache' not in pane(11.8)
    assert 'Group: Project' in pane(12.8)
    assert 'Task shortcuts' in pane(16.8)
    assert 'Task shortcuts' not in pane(18.4)
    gaps = [b['time'] - a['time'] for a, b in zip(recording, recording[1:])]
    assert max(gaps) < 0.2, 'Capture stalled; repeat the recording on an idle host'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/assets/session-tour.gif')
    parser.add_argument('--frames-dir', type=Path, help='Keep captures, frames and timing for review')
    args = parser.parse_args()
    binary = ROOT / 'target/debug/codex-fleet'
    if not binary.is_file():
        parser.error('Build the application first: cargo build --locked')
    for command in ('tmux', 'magick', 'ffmpeg'):
        if not shutil.which(command):
            parser.error(f'Required command is missing: {command}')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.frames_dir:
        args.frames_dir.mkdir(parents=True, exist_ok=True)
    workspace = nullcontext(str(args.frames_dir)) if args.frames_dir else tempfile.TemporaryDirectory(prefix='fleet-frames-')
    with workspace as directory, tempfile.TemporaryDirectory(prefix='fleet-record-') as session:
        frames = Path(directory)
        tmux_args = ['tmux', '-S', str(Path(session) / 'tmux.sock')]
        env = dict(os.environ)
        env.pop('NO_COLOR', None)
        env['TERM'] = 'xterm-256color'

        def tmux(*command, check=True):
            return subprocess.run(tmux_args + list(command), env=env, check=check,
                                  capture_output=True, text=True).stdout

        recording, sent = [], []
        try:
            tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'demo', '-x', str(COLS),
                 '-y', str(ROWS), str(binary), '--demo')
            time.sleep(0.4)
            print(f'Recording {DURATION}s at {FPS} fps with live keystrokes...', flush=True)
            start = time.monotonic()
            pending = iter(EVENTS)
            event = next(pending, None)
            for index in range(FPS * DURATION):
                time.sleep(max(0, start + index / FPS - time.monotonic()))
                elapsed = time.monotonic() - start
                while event and event[0] <= elapsed:
                    tmux('send-keys', '-t', 'demo', event[1])
                    sent.append((time.monotonic() - start, *event[1:]))
                    event = next(pending, None)
                capture = tmux('capture-pane', '-p', '-e', '-t', 'demo')
                recording.append({'time': time.monotonic() - start, 'capture': capture})
        finally:
            tmux('kill-server', check=False)
        validate_recording(recording)
        assert len(sent) == len(EVENTS), 'A scheduled key was not sent'
        print(f'Captured {len(recording)} frames; typing and UI transitions verified.', flush=True)
        (frames / 'recording.json').write_text(json.dumps({'fps': FPS, 'events': sent, 'frames': recording}, indent=2))

        # Rendering happens after recording so it cannot stall input or omit
        # transient states. Reuse identical rasters during reading pauses.
        unique = {}
        for index, frame in enumerate(recording):
            svg = frame_svg(frame['capture'])
            path = frames / f'frame-{index:04d}.png'
            path.unlink(missing_ok=True)
            path.with_suffix('.svg').write_text(svg)
            original = unique.setdefault(svg, path)
            frame['path'], frame['original'] = path, original

        def rasterize(path):
            subprocess.run(['magick', '-background', BACKGROUND, str(path.with_suffix('.svg')),
                            '-alpha', 'remove', str(path)], check=True)

        print(f'Rendering {len(unique)} distinct frames...', flush=True)
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(rasterize, unique.values()))
        for frame in recording:
            if frame['path'] != frame['original']:
                os.link(frame['original'], frame['path'])
        # A shared palette and no dithering keep static text from shimmering.
        source = str(frames / 'frame-%04d.png')
        palette = str(frames / 'palette.png')
        run = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-framerate', str(FPS), '-i', source]
        subprocess.run(run + ['-vf', 'palettegen=stats_mode=diff:max_colors=128',
                              '-frames:v', '1', '-update', '1', palette], check=True)
        result = frames / 'result.gif'
        subprocess.run(run + ['-i', palette, '-lavfi', 'paletteuse=dither=none:diff_mode=rectangle',
                              '-loop', '0', str(result)], check=True)
        args.output.write_bytes(result.read_bytes())
    print(f'Saved {args.output} ({args.output.stat().st_size:,} bytes)')


if __name__ == '__main__':
    main()
