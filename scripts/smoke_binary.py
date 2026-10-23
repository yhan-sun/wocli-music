"""Exercise an extracted binary with no Python, uv or system mpv on its PATH."""

import argparse
import csv
import json
import os
import re
import select
import struct
import subprocess
import sys
import tempfile
import time
import wave
from pathlib import Path

is_windows = sys.platform == "win32"
if not is_windows:
    import fcntl
    import pty
    import termios


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument(
        "--docker", action="store_true", help="Test in a pristine Ubuntu 22.04 container"
    )
    args = parser.parse_args()
    executable = args.executable.resolve()
    with tempfile.TemporaryDirectory(prefix="wocli-binary-smoke-") as temporary:
        root = Path(temporary).resolve()
        environment = {
            key: value
            for key, value in os.environ.items()
            if key
            not in ("PYTHONPATH", "PYTHONHOME", "WOCLI_MPV", "SSL_CERT_FILE", "CURL_CA_BUNDLE")
        }
        clean_path = "C:\\Windows\\System32;C:\\Windows" if is_windows else "/no-runtime-installed"
        environment.update(
            PATH=clean_path,
            WOCLI_HOME=str(root / "data"),
            WOCLI_AUDIO_OUTPUT="null",
            TERM="xterm-256color",
        )
        prefix = []
        container = None
        song = root / "test.wav"
        with wave.open(str(song), "wb") as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(8000)
            audio.writeframes(b"\0\0" * 8000 * 30)
        song.with_suffix(".lrc").write_text(
            "[00:00]Portable bundle test\n[00:05]Local lyrics", encoding="utf8"
        )
        if args.docker:
            container = subprocess.check_output(
                [
                    "docker",
                    "run",
                    "--detach",
                    "--rm",
                    "--user",
                    f"{os.getuid()}:{os.getgid()}",
                    "--network",
                    "none",
                    "-v",
                    f"{executable.parent}:/app:ro",
                    "-v",
                    f"{root}:/work",
                    "ubuntu:22.04",
                    "sleep",
                    "300",
                ],
                text=True,
            ).strip()
            prefix = [
                "docker",
                "exec",
                "-e",
                "PATH=/no-runtime-installed",
                "-e",
                "WOCLI_HOME=/work/data",
                "-e",
                "WOCLI_AUDIO_OUTPUT=null",
                container,
            ]
            executable = Path("/app/wocli")
            song = Path("/work/test.wav")

        def cli(*arguments):
            if is_windows:
                stdout_file = root / "stdout.tmp"
                stderr_file = root / "stderr.tmp"
                with stdout_file.open("w", encoding="utf8") as out, stderr_file.open("w", encoding="utf8") as err:
                    result = subprocess.run(
                        [*prefix, str(executable), *arguments],
                        cwd=root,
                        env=environment,
                        stdin=subprocess.DEVNULL,
                        stdout=out,
                        stderr=err,
                        timeout=30,
                    )
                stdout = stdout_file.read_text(encoding="utf8", errors="replace").strip()
                stderr = stderr_file.read_text(encoding="utf8", errors="replace").strip()
                if result.returncode:
                    diagnostics = []
                    for name in ("service-rust.log",):
                        log = root / "data" / name
                        if log.is_file():
                            diagnostics.append(name + ":\n" + log.read_text(errors="replace")[-4000:])
                    raise RuntimeError(
                        f"{arguments}: {stderr}\n{stdout}\n" + "\n".join(diagnostics)
                    )
                return stdout
            result = subprocess.run(
                [*prefix, str(executable), *arguments],
                cwd=root,
                env=None if args.docker else environment,
                text=True,
                capture_output=True,
                timeout=30,
            )
            if result.returncode:
                diagnostics = []
                for name in ("service-rust.log",):
                    log = root / "data" / name
                    if log.is_file():
                        diagnostics.append(name + ":\n" + log.read_text(errors="replace")[-4000:])
                raise RuntimeError(
                    f"{arguments}: {result.stderr}\n{result.stdout}\n" + "\n".join(diagnostics)
                )
            return result.stdout.strip()

        try:
            print(cli("--version"), flush=True)
            tools = json.loads(cli("mcp", "tools"))["tools"]
            assert len(tools) == 17
            readonly_tools = json.loads(cli("mcp", "tools", "--read-only"))["tools"]
            assert len(readonly_tools) == 8 and all(t["annotations"]["readOnlyHint"] for t in readonly_tools)
            mcp_config = json.loads(cli("mcp", "config"))["mcpServers"]["wocli-music"]
            assert mcp_config["args"][:2] == ["mcp", "serve"]
            guide = json.loads(cli("mcp", "prompt", "--json"))
            assert guide["config"]["mcpServers"]["wocli-music"] == mcp_config
            assert cli("mcp", "prompt") == guide["prompt"]
            assert "get_playback_status" in guide["prompt"]
            # Exercise the packaged stdio transport without a client runtime or network.
            initialize = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "bundle-smoke", "version": "1"},
            }}
            initialized = {"jsonrpc": "2.0", "method": "notifications/initialized"}
            mcp_command = [*prefix, str(executable), "mcp", "serve", "--read-only"]
            if args.docker:
                # docker exec otherwise discards the client's JSON-RPC stdin.
                mcp_command.insert(2, "--interactive")
            mcp = subprocess.run(
                mcp_command,
                cwd=root, env=None if args.docker else environment,
                text=True, input=json.dumps(initialize) + "\n" + json.dumps(initialized) + "\n",
                capture_output=True, timeout=15,
            )
            assert mcp.returncode == 0, mcp.stderr
            assert not mcp.stderr.strip(), mcp.stderr
            replies = [json.loads(line) for line in mcp.stdout.splitlines()]
            assert any(reply.get("id") == 1 and reply.get("result", {}).get("protocolVersion") == "2025-11-25" for reply in replies), replies
            doctor = json.loads(cli("doctor", "--json"))
            assert doctor["runtime"] == "native" and doctor["engine"] == "rodio/symphonia", doctor
            state = json.loads(cli("play", str(song), "--json"))
            assert state["phase"] == "playing", state
            # The short-lived CLI has exited; the detached frozen service and mpv survive.
            assert json.loads(cli("status", "--json"))["phase"] == "playing"
            assert json.loads(cli("pause", "--json"))["phase"] == "paused"
            cli("seek", "+3")
            for _ in range(30):
                state = json.loads(cli("status", "--json"))
                if state["position"] >= 2.8:
                    break
                time.sleep(0.1)
            assert state["phase"] == "paused" and state["position"] >= 2.8
            cli("like")
            assert len(json.loads(cli("favorites", "--json"))) == 1
            cli("playlist", "create", "Portable")
            cli("playlist", "add", "Portable", "--id", state["track"]["id"])
            assert len(json.loads(cli("playlist", "show", "Portable", "--json"))) == 1
            # Portable playlist transfer uses only the native binary and SQLite.
            portable_csv = root / "portable.csv"
            with portable_csv.open("w", encoding="utf-8-sig", newline="") as stream:
                writer = csv.writer(stream)
                writer.writerow(["title", "artist", "path"])
                writer.writerow(["Portable song", "Portable artist", str(song)])
                writer.writerow(["Missing song", "Unknown artist", str(root / "missing.wav")])
            portable_input = "/work/portable.csv" if args.docker else str(portable_csv)
            preview = json.loads(cli("playlist", "import", portable_input, "--name", "Migrated", "--offline", "--dry-run", "--json"))
            assert preview["summary"]["matched"] == 1 and preview["summary"]["pending"] == 1, preview
            assert all(item["name"] != "Migrated" for item in json.loads(cli("playlist", "list", "--json")))
            imported = json.loads(cli("playlist", "import", portable_input, "--name", "Migrated", "--offline", "--json"))
            assert imported["summary"]["total"] == 2, imported
            assert len(json.loads(cli("playlist", "show", "Migrated", "--json"))) == 1
            exported = root / "portable.wocli.json"
            portable_output = "/work/portable.wocli.json" if args.docker else str(exported)
            cli("playlist", "export", "Migrated", "--output", portable_output)
            reimported = json.loads(cli("playlist", "import", portable_output, "--offline", "--json"))
            assert reimported["name"] == "Migrated (导入 2)" and reimported["summary"]["total"] == 2, reimported
            cli("mode", "shuffle")
            assert json.loads(cli("status", "--json"))["mode"] == "shuffle"
            assert "Portable bundle test" in json.loads(cli("lyrics", "--json"))["lyrics"]
            if not args.docker and not is_windows:
                master, slave = pty.openpty()
                process = None
                try:
                    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 26, 80, 0, 0))
                    process = subprocess.Popen(
                        [str(executable)],
                        stdin=slave,
                        stdout=slave,
                        stderr=slave,
                        cwd=root,
                        env=environment,
                        start_new_session=True,
                    )
                    transcript = bytearray()
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline:
                        ready, _, _ = select.select([master], [], [], 0.1)
                        if ready:
                            transcript.extend(os.read(master, 65536))
                    def pump_screen(seconds=0.15):
                        until = time.monotonic() + seconds
                        while time.monotonic() < until:
                            ready, _, _ = select.select([master], [], [], 0.05)
                            if ready:
                                transcript.extend(os.read(master, 65536))
                    # Exercise actual terminal byte decoding and daemon acknowledgements.
                    # Shift+B, Ctrl+B, then F12; Esc restores after each.
                    for key in [b"B", b"\x02", b"\x1b[24~"]:
                        os.write(master, key)
                        until = time.monotonic() + 4
                        while True:
                            pump_screen()
                            state = json.loads(cli("status", "--json"))
                            if state["boss"]:
                                break
                            assert time.monotonic() < until, (key, state)
                        assert state["phase"] == "paused"
                        # More than two status polls must not undo camouflage.
                        pump_screen(0.8)
                        assert json.loads(cli("status", "--json"))["boss"]
                        os.write(master, b"\x1b")
                        until = time.monotonic() + 4
                        while True:
                            pump_screen()
                            state = json.loads(cli("status", "--json"))
                            if not state["boss"]:
                                break
                            assert time.monotonic() < until, state
                        assert state["phase"] == "paused"
                    assert b"workspace-engine" in transcript
                    deadline = time.monotonic()
                    os.write(master, b"q")
                    while process.poll() is None and time.monotonic() < deadline + 10:
                        ready, _, _ = select.select([master], [], [], 0.1)
                        if ready:
                            transcript.extend(os.read(master, 65536))
                    assert process.wait(timeout=2) == 0, transcript.decode(errors="replace")[-4000:]
                    assert "收藏到" in re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", transcript.decode(errors="replace")), transcript.decode(errors="replace")[-3000:]
                finally:
                    if process and process.poll() is None:
                        process.kill()
                        process.wait()
                    os.close(master)
                    os.close(slave)
            assert json.loads(cli("status", "--json"))["phase"] == "paused"
            print(
                "PASS: native decoding, detached service, playback, seek, lyrics, collections, mode and TUI",
                flush=True,
            )
        finally:
            try:
                cli("daemon", "stop")
            finally:
                if container:
                    subprocess.run(
                        ["docker", "stop", container], check=True, stdout=subprocess.DEVNULL
                    )


if __name__ == "__main__":
    main()
