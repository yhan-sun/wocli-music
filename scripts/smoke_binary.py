"""Exercise an extracted binary with no Python, uv or system mpv on its PATH."""

import argparse
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
