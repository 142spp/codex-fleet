#!/usr/bin/env python3
"""Real PTY regression test. Requires tmux and Python websockets; no Codex account.

Build with cargo build --locked, then run python3 tests/terminal_smoke.py.
Uses an isolated tmux socket and a read-only mock server. Never contacts live hosts.
"""
import asyncio
import json
from pathlib import Path
import subprocess
import tempfile

import websockets

ROOT = Path(__file__).resolve().parents[1]
THREAD = {"id": "fixture-thread", "name": "한글 terminal fixture", "cwd": "/tmp",
          "updatedAt": 1, "status": {"type": "idle"}, "model": "fixture-model",
          "reasoningEffort": "high", "gitInfo": {"branch": "fixture-branch"}}


async def main():
    with tempfile.TemporaryDirectory(prefix="fleet-pty-") as directory:
        temp = Path(directory)
        socket = temp / "rpc.sock"
        tmux_socket = str(temp / "tmux.sock")
        calls = []
        archived = False
        fail_archive = True

        async def serve(ws):
            nonlocal archived, fail_archive
            while True:
                try:
                    text = await ws.recv()
                except websockets.exceptions.ConnectionClosed:
                    return
                request = json.loads(text)
                method = request["method"]
                calls.append(method)
                if "id" not in request:
                    continue
                if method == "initialize":
                    result = {}
                elif method == "thread/list":
                    result = {"data": [] if archived else [THREAD], "nextCursor": None}
                elif method == "thread/loaded/list":
                    result = {"data": [], "nextCursor": None}
                elif method == "thread/read":
                    result = {"thread": THREAD}
                elif method == "thread/turns/list":
                    result = {"data": [{"status": "completed", "items": [
                        {"type": "agentMessage", "text": "정상 응답 fixture"}]}]}
                elif method == "thread/archive":
                    assert request["params"]["threadId"] == "fixture-thread"
                    if fail_archive:
                        fail_archive = False
                        await ws.send(json.dumps({"id":request["id"], "error":{"code":-32000,"message":"fixture denied archive"}}))
                        continue
                    archived = True
                    result = {}
                else:
                    raise AssertionError(f"Unexpected/mutating RPC: {method}")
                await ws.send(json.dumps({"id": request["id"], "result": result}))

        fake = temp / "fake-codex"
        record = temp / "launch.json"
        fake.write_text("#!/usr/bin/env python3\nimport json,sys\n"
                        f"open({str(record)!r},'w').write(json.dumps(sys.argv[1:]))\n"
                        "print('FLEET_CHILD_READY',flush=True)\n"
                        "command=input()\nsys.exit(23 if command=='fail' else 0)\n")
        fake.chmod(0o700)
        config = temp / "config.toml"
        config.write_text(f'poll_seconds = 1\n[[hosts]]\nid = "fixture"\nname = "Fixture"\n'
                          f'socket = {json.dumps(str(socket))}\ncodex = {json.dumps(str(fake))}\n')

        def tmux(*args, check=True):
            return subprocess.run(["tmux", "-S", tmux_socket, *args], check=check,
                                  capture_output=True, text=True).stdout

        async def wait_for(needle, absent=False):
            for _ in range(200):
                pane = tmux("capture-pane", "-p", "-t", "test")
                if (needle in pane) != absent:
                    return pane
                await asyncio.sleep(0.05)
            raise AssertionError(f"Timed out waiting for {needle!r}:\n{pane}")

        async with websockets.unix_serve(serve, str(socket)):
            try:
                tmux("new-session", "-d", "-s", "test", "-x", "110", "-y", "32",
                     str(ROOT / "target/debug/codex-fleet"), "--config", str(config),
                     "--state-file", str(temp / "state.json"))
                tmux("set-option", "-t", "test", "remain-on-exit", "on")
                await wait_for("한글 terminal fixture")
                await wait_for("Task details")
                await wait_for("fixture-model")
                assert "[Fixture]" not in tmux("capture-pane", "-p", "-t", "test")
                tmux("send-keys", "-t", "test", "d")
                await wait_for("Recent conversation")
                await wait_for("정상 응답 fixture")
                tmux("send-keys", "-t", "test", "Escape")
                await wait_for("Recent conversation", absent=True)
                for response in ["return", "fail"]:
                    tmux("send-keys", "-t", "test", "Enter")
                    await wait_for("FLEET_CHILD_READY")
                    tmux("send-keys", "-t", "test", "-l", response)
                    tmux("send-keys", "-t", "test", "Enter")
                    await wait_for("Codex Fleet")
                    assert json.loads(record.read_text()) == ["resume", "--remote", f"unix://{socket}", "--cd", "/tmp", "--", "fixture-thread"]
                await wait_for("23")
                tmux("send-keys", "-t", "test", "a")
                await wait_for("Archive failed:")
                assert "한글 terminal fixture" in tmux("capture-pane", "-p", "-t", "test")
                tmux("send-keys", "-t", "test", "a")
                await wait_for("Archived.")
                await wait_for("All 0 ")
                tmux("resize-window", "-t", "test", "-x", "42", "-y", "12")
                await wait_for("Codex Fleet")
                await wait_for("Task details", absent=True)
                tmux("resize-window", "-t", "test", "-x", "140", "-y", "40")
                await wait_for("Task details")
                tmux("send-keys", "-t", "test", "q")
                for _ in range(100):
                    state = tmux("display-message", "-p", "-t", "test", "#{pane_dead}:#{pane_dead_status}")
                    if state.strip() == "1:0":
                        break
                    await asyncio.sleep(0.05)
                else:
                    raise AssertionError(f"TUI did not exit cleanly: {state}")
                assert "thread/turns/list" in calls
                assert set(calls) <= {"initialize", "initialized", "thread/list", "thread/loaded/list", "thread/read", "thread/turns/list", "thread/archive"}
                assert calls.count("thread/archive") == 2
                print("PASS: device sections, Korean detail, two Codex handoffs, archive failure/success, resize, clean exit")
            finally:
                tmux("kill-server", check=False)


if __name__ == "__main__":
    asyncio.run(main())
