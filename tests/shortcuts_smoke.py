#!/usr/bin/env python3
"""All task shortcuts: isolated PTY, mock Codex, and a real disposable Git repo."""
import asyncio
import copy
import json
from pathlib import Path
import subprocess
import tempfile

import websockets

ROOT = Path(__file__).resolve().parents[1]


async def main():
    with tempfile.TemporaryDirectory(prefix="fleet-keys-") as directory:
        temp = Path(directory)
        repo = temp / "repo"
        repo.mkdir()
        def git(*args):
            return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout
        git("init")
        (repo / "tracked.txt").write_text("baseline\n")
        git("add", ".")
        git("-c", "user.name=Fleet Test", "-c", "user.email=fleet@example.invalid", "commit", "-m", "fixture")
        (repo / "tracked.txt").write_text("dirty work stays here\n")
        threads = {"root": {"id": "root", "name": "root task", "cwd": str(repo),
                            "status": {"type": "idle"}, "updatedAt": 1, "isPinned": False}}
        calls = []
        sequence = 0
        async def serve(ws):
            nonlocal sequence
            while True:
                try:
                    request = json.loads(await ws.recv())
                except websockets.exceptions.ConnectionClosed:
                    return
                method = request["method"]
                params = request.get("params", {})
                calls.append((method, params))
                if "id" not in request:
                    continue
                if method == "initialize":
                    result = {}
                elif method == "thread/list":
                    result = {"data": list(threads.values()), "nextCursor": None}
                elif method == "thread/loaded/list":
                    result = {"data": [], "nextCursor": None}
                elif method == "thread/read":
                    thread = threads.get(params["threadId"])
                    if thread is None:
                        await ws.send(json.dumps({"id": request["id"], "error": {
                            "code": -32600, "message": "Thread not found"}}))
                        continue
                    result = {"thread": thread}
                elif method == "thread/turns/list":
                    t = threads[params["threadId"]]
                    result = {"data": [{"id": "active-turn", "status": "inProgress", "items": []}]
                              if t["status"]["type"] == "active" else []}
                elif method == "thread/name/set":
                    threads[params["threadId"]]["name"] = params["name"]
                    result = {}
                elif method == "thread/metadata/update":
                    threads[params["threadId"]]["isPinned"] = params["isPinned"]
                    result = {"thread": threads[params["threadId"]]}
                elif method == "turn/interrupt":
                    assert params["turnId"] == "active-turn"
                    threads[params["threadId"]]["status"] = {"type": "idle"}
                    result = {}
                elif method in ("thread/start", "thread/fork"):
                    sequence += 1
                    t = copy.deepcopy(threads[params["threadId"]] if method == "thread/fork" else threads["root"])
                    t.update(id=f"created-{sequence}", name=f"created task {sequence}", updatedAt=sequence + 1)
                    if method == "thread/start":
                        t["cwd"] = params["cwd"]
                    threads[t["id"]] = t
                    result = {"thread": t}
                elif method == "thread/delete":
                    del threads[params["threadId"]]
                    result = {}
                else:
                    raise AssertionError(f"Unexpected RPC: {method}")
                await ws.send(json.dumps({"id": request["id"], "result": result}))

        fake = temp / "codex"
        launches = temp / "launches.jsonl"
        fake.write_text("#!/usr/bin/env python3\nimport json,sys\n"
                        f"with open({str(launches)!r},'a') as f:f.write(json.dumps(sys.argv[1:])+'\\n')\n"
                        "print('FLEET_CHILD_READY',flush=True)\ninput()\n")
        fake.chmod(0o700)
        socket = temp / "rpc.sock"
        config = temp / "config.toml"
        config.write_text(f'poll_seconds = 1\n[[hosts]]\nid="fixture"\nname="Fixture"\n'
                          f'socket={json.dumps(str(socket))}\ncodex={json.dumps(str(fake))}\n')
        tmux_socket = str(temp / "tmux")
        def tmux(*args, check=True):
            return subprocess.run(["tmux", "-S", tmux_socket, *args], check=check, capture_output=True, text=True).stdout
        def send(*keys):
            tmux("send-keys", "-t", "test", *keys)
        async def wait(text, absent=False):
            for _ in range(300):
                pane = tmux("capture-pane", "-p", "-t", "test")
                if (text in pane) != absent:
                    return pane
                await asyncio.sleep(0.03)
            raise AssertionError(f"Timeout {text!r}\n{pane}")
        async def return_from_cli():
            await wait("FLEET_CHILD_READY")
            send("-l", "return")
            send("Enter")
            await wait("? help  ↑/↓ move  enter open")
        async def select_root():
            send("F3")
            await wait("▏")
            send("-l", "root")
            await wait("/root▏")
            send("Enter")
            await wait("▏", absent=True)
        command = [str(ROOT / "target/debug/codex-fleet"), "--config", str(config), "--state-file", str(temp / "state.json")]
        async with websockets.unix_serve(serve, str(socket)):
            try:
                tmux("new-session", "-d", "-s", "test", "-x", "140", "-y", "40", *command)
                tmux("set-option", "-t", "test", "remain-on-exit", "on")
                await wait("root task")
                send("?")
                await wait("Task shortcuts")
                footer=tmux("capture-pane", "-p", "-t", "test").rstrip().splitlines()[-1]
                assert footer.strip()=="esc back    ↑/↓ scroll on narrow screens",footer
                for name in ["New worktree", "Fork", "Rename", "Stop", "Hide / unhide", "Delete", "Pin / unpin"]:
                    assert name in tmux("capture-pane", "-p", "-t", "test")
                send("Escape")
                await wait("Task shortcuts", absent=True)
                send("r")
                await wait("Rename task")
                send("C-u")
                send("-l", "renamed root")
                send("Enter")
                await wait("Rename completed")
                assert threads["root"]["name"] == "renamed root"
                send("p")
                await wait("Pinned locally")
                assert ["fixture", "root"] in json.loads((temp / "state.json").read_text())["pinned"]
                send("h")
                await wait("All 0 ")
                assert "root" in threads
                send("q")
                await asyncio.sleep(0.1)
                tmux("respawn-pane", "-k", "-t", "test", *command)
                await wait("All 0 ")
                send("H")
                await wait("renamed root")
                send("h")
                await wait("Task restored")
                send("H")
                await wait("renamed root")
                threads["root"]["status"] = {"type": "active"}
                send("F5")
                await wait("Working")
                send("x")
                await wait("Stop requested")
                assert threads["root"]["status"]["type"] == "idle"
                send("o")
                await return_from_cli()
                send("n")
                await wait("New task")
                send("C-u")
                send("-l", str(repo))
                send("Enter")
                await return_from_cli()
                assert "created-1" in threads
                send("f")
                await return_from_cli()
                assert any(m == "thread/fork" and p["threadId"] == "created-1" for m, p in calls)
                send("BSpace")
                await wait("Permanently delete")
                send("n")
                await wait("Permanently delete", absent=True)
                assert "created-2" in threads
                send("BSpace")
                await wait("Permanently delete")
                send("y")
                await wait("Delete completed")
                assert "created-2" not in threads
                await select_root()
                send("w")
                await wait("New worktree")
                send("C-u")
                send("-l", "codex-fleet/fixture")
                send("Enter")
                await return_from_cli()
                assert "codex-fleet/fixture" in git("branch", "--list")
                assert (repo / "tracked.txt").read_text() == "dirty work stays here\n"
                assert ".codex-fleet-worktrees" in threads["created-3"]["cwd"]
                assert Path(threads["created-3"]["cwd"], "tracked.txt").read_text() == "baseline\n"
                send("q")
                expected = {"thread/start", "thread/fork", "thread/name/set", "thread/delete", "turn/interrupt"}
                assert expected <= {m for m, _ in calls}
                print("PASS: all task shortcuts, hide persistence, cancellation, exact RPC targets, real worktree and dirty-file preservation")
            finally:
                tmux("kill-server", check=False)


if __name__ == "__main__":
    asyncio.run(main())
