#!/usr/bin/env python3
"""Opt-in live validation: create an empty test thread per host, open/exit it,
then delete ONLY the created test thread. Sends no prompts. Requires tmux and
Python websockets. Run after cargo build, with --config fleet.local.toml.
"""
import argparse
import asyncio
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import tomllib

import websockets

ROOT = Path(__file__).resolve().parents[1]


async def check_host(host, directory):
    temp = Path(directory)
    name = host["id"]
    codex = host.get("codex", "codex")
    ssh = host.get("ssh")
    args = [codex, "app-server", "daemon", "version"]
    if ssh:
        args = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=7", "--", ssh, shlex.join(args)]
    discovery = await asyncio.create_subprocess_exec(*args, stdout=asyncio.subprocess.PIPE)
    output, _ = await asyncio.wait_for(discovery.communicate(), 30)
    if discovery.returncode:
        raise RuntimeError(f"{name}: daemon discovery failed")
    version = json.loads(output)
    remote_socket = host.get("socket", version["socketPath"])
    socket = Path(remote_socket)
    tunnel = None
    thread = None
    tmux_socket = str(temp / f"{name}.tmux")

    def tmux(*command, check=True):
        return subprocess.run(["tmux", "-S", tmux_socket, *command], check=check,
                              capture_output=True, text=True).stdout

    async def wait_for(text, absent=False):
        for _ in range(600):
            pane = tmux("capture-pane", "-p", "-t", "live")
            if (text in pane) != absent:
                return pane
            await asyncio.sleep(0.1)
        raise AssertionError(f"{name}: timeout waiting for {text!r}:\n{pane}")

    try:
        if ssh:
            socket = temp / f"{name}.sock"
            tunnel = await asyncio.create_subprocess_exec("ssh", "-N", "-o", "BatchMode=yes",
                "-o", "ExitOnForwardFailure=yes", "-o", "ConnectTimeout=7", "-L",
                f"{socket}:{remote_socket}", ssh)
            for _ in range(200):
                if socket.exists():
                    break
                if tunnel.returncode is not None:
                    raise RuntimeError(f"{name}: SSH tunnel failed")
                await asyncio.sleep(0.1)
            else:
                raise TimeoutError(f"{name}: no forwarded socket")
        async with websockets.unix_connect(str(socket), uri="ws://localhost/rpc") as ws:
            request_id = 0

            async def request(method, params):
                nonlocal request_id
                request_id += 1
                await ws.send(json.dumps({"id": request_id, "method": method, "params": params}))
                while True:
                    result = json.loads(await asyncio.wait_for(ws.recv(), 30))
                    if result.get("id") == request_id:
                        if "error" in result:
                            raise RuntimeError(result["error"])
                        return result["result"]

            await request("initialize", {"clientInfo": {"name": "codex_fleet_live_test", "version": "0.1.0"}})
            await ws.send('{"method":"initialized","params":{}}')
            created = await request("thread/start", {"cwd": "/tmp", "sandbox": "read-only", "approvalPolicy": "on-request"})
            thread = created["thread"]["id"]
            try:
                await request("thread/name/set", {"threadId": thread, "name": "Codex Fleet temporary navigation test"})
                forked = await request("thread/fork", {"threadId": thread, "excludeTurns": True})
                fork_id = forked["thread"]["id"]
                assert fork_id != thread
                await request("thread/delete", {"threadId": fork_id})
                isolated = temp / f"{name}.toml"
                selected = dict(host, socket=remote_socket)
                isolated.write_text("poll_seconds = 1\ntimeout_seconds = 20\n[[hosts]]\n" +
                    "\n".join(f"{key} = {json.dumps(value)}" for key, value in selected.items()) + "\n")
                tmux("new-session", "-d", "-s", "live", "-x", "110", "-y", "35",
                     str(ROOT / "target/debug/codex-fleet"), "--config", str(isolated),
                     "--state-file", str(temp / f"{name}.state.json"))
                tmux("set-option", "-t", "live", "remain-on-exit", "on")
                await wait_for("Connected 1/1")
                tmux("send-keys", "-t", "live", "/")
                await wait_for("/▏")
                tmux("send-keys", "-t", "live", "-l", thread)
                await wait_for("All 1 ")
                tmux("send-keys", "-t", "live", "Enter")
                await wait_for("▏", absent=True)
                tmux("send-keys", "-t", "live", "p")
                await wait_for("Pinned locally")
                tmux("send-keys", "-t", "live", "p")
                await wait_for("Unpinned locally")
                tmux("send-keys", "-t", "live", "Enter")
                pane = await wait_for("Ask Codex to do anything")
                assert "OpenAI Codex" in pane, pane
                # No prompt: exit via the CLI's own command, then verify dashboard recovery.
                tmux("send-keys", "-t", "live", "-l", "/quit")
                await wait_for("/quit")
                tmux("send-keys", "-t", "live", "Enter")
                # The CLI itself may display the chat title containing "Codex Fleet".
                # Wait for Fleet's unique footer, not that ambiguous title.
                await wait_for("? help  ↑/↓ move  enter open")
                await wait_for("temporary navigation test")
                tmux("send-keys", "-t", "live", "d")
                await wait_for("Recent conversation")
                await wait_for(thread)
                tmux("send-keys", "-t", "live", "Escape")
                await wait_for("Recent conversation", absent=True)
                turns = await request("thread/turns/list", {"threadId": thread, "limit": 1})
                assert turns["data"] == [], "A prompt was unexpectedly sent"
                tmux("send-keys", "-t", "live", "a")
                await wait_for("Archived.")
                await wait_for("All 0 ")
                archived = await request("thread/list", {"archived": True, "limit": 100, "sortKey": "updated_at", "sortDirection": "desc"})
                assert any(t["id"] == thread for t in archived["data"]), "Thread was not archived on the owning server"
                tmux("send-keys", "-t", "live", "q")
                for _ in range(50):
                    state = tmux("display-message", "-p", "-t", "live", "#{pane_dead}:#{pane_dead_status}").strip()
                    if state == "1:0":
                        break
                    await asyncio.sleep(0.1)
                else:
                    raise AssertionError(f"{name}: dashboard exit state {state}")
                print(f"PASS {name}: device section → exact session → Codex → detail → archive → clean exit; zero turns", flush=True)
            finally:
                tmux("kill-server", check=False)
                if thread:
                    await request("thread/delete", {"threadId": thread})
                    print(f"CLEANED {name}: temporary thread {thread}", flush=True)
    finally:
        if tunnel and tunnel.returncode is None:
            tunnel.terminate()
            await tunnel.wait()


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    config = tomllib.loads(args.config.read_text())
    with tempfile.TemporaryDirectory(prefix="fleet-live-") as directory:
        for host in config["hosts"]:
            await check_host(host, directory)


if __name__ == "__main__":
    asyncio.run(main())
