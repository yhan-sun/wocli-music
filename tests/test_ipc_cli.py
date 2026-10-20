import asyncio
import json
import sys
from pathlib import Path

import pytest

from wocli_music.cli import make_parser
from wocli_music.client import Client
from wocli_music.models import MusicError
from wocli_music.player import Mpv


@pytest.fixture
def fake_mpv(tmp_path, monkeypatch):
    executable = tmp_path / "mpv"
    source = Path(__file__).with_name("fake_mpv.py").read_text()
    executable.write_text(f"#!{sys.executable}\n" + source)
    executable.chmod(0o700)
    monkeypatch.setenv("WOCLI_MPV", str(executable))
    return executable


async def test_mpv_real_process_ipc_and_cleanup(paths, fake_mpv):
    events = []
    player = Mpv(paths, events.append)
    try:
        await player.load("中文 path with spaces.mp3", 45)
        process = player.process
        await player.command("set_property", "pause", True)
        await player.command("seek", 5, "relative+exact")
        assert player.properties["pause"] is True
        assert player.properties["time-pos"] == 5
        assert player.properties["volume"] == 45
        await player.load("second.mp3", 30)
        assert player.process is process
        assert any(event.get("event") == "file-loaded" for event in events)
        with pytest.raises(MusicError, match="无法读取"):
            await player.load("fail", 30)
    finally:
        await player.close()
    assert process.returncode == 0
    assert not player.socket.exists()


async def test_concurrent_daemon_start_and_state_persist_after_shutdown(paths):
    clients = [Client(paths), Client(paths)]
    try:
        results = await asyncio.gather(*(client.request("status") for client in clients))
        assert all(state["phase"] == "stopped" for state in results)
        await clients[0].request("volume", value=35)
        assert (await clients[1].request("status"))["volume"] == 35
        with pytest.raises(MusicError, match="未知操作"):
            await clients[1].request("invalid_operation")
    finally:
        await clients[0].request("shutdown", autostart=False)
        for _ in range(100):
            if not paths.socket.exists():
                break
            await asyncio.sleep(0.02)
    assert not paths.socket.exists()
    assert await clients[0].request("status", autostart=False) is None


async def run_cli(*arguments):
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "wocli_music",
        *arguments,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    output, error = await asyncio.wait_for(process.communicate(), 15)
    return process.returncode, output.decode(), error.decode()


async def test_cli_local_play_pause_queue_and_json(paths, fake_mpv, tmp_path):
    media = tmp_path / "song.wav"
    media.write_bytes(b"RIFF test")
    client = Client(paths)
    try:
        code, output, error = await run_cli("play", str(media), "--json")
        assert code == 0, error
        state = json.loads(output)
        assert state["phase"] == "playing"
        assert state["quality"] == "local"
        code, output, _ = await run_cli("pause", "--json")
        assert code == 0
        assert json.loads(output)["phase"] == "paused"
        code, output, _ = await run_cli("like", "--json")
        assert code == 0
        assert json.loads(output)["liked"] is True
        code, output, _ = await run_cli("favorites", "--json")
        assert json.loads(output)[0]["path"] == str(media)
        code, output, _ = await run_cli("queue", "add", str(media), "--json")
        assert len(json.loads(output)["queue"]) == 2
        code, _, error = await run_cli("volume", "101", "--json")
        assert code == 1
        assert json.loads(error)["ok"] is False
        code, output, error = await run_cli("stop", "--json")
        assert json.loads(output)["phase"] == "stopped"
    finally:
        await client.request("shutdown", autostart=False)
        for _ in range(100):
            if not paths.socket.exists():
                break
            await asyncio.sleep(0.02)


@pytest.mark.parametrize(
    "argv",
    [
        ["--json", "status"],
        ["status", "--json"],
        ["--json", "queue", "list"],
        ["queue", "--json", "list"],
        ["queue", "list", "--json"],
    ],
)
def test_json_flag_works_at_each_command_depth(argv):
    assert make_parser().parse_args(argv).json is True


async def test_cli_rename_delete_collection_and_duplicate_name_error(paths, tracks):
    from wocli_music.store import Store

    store = Store(paths)
    store.add_to_playlist("工作", tracks[0])
    store.add_to_playlist("备份", tracks[0])
    code, output, error = await run_cli("playlist", "rename", "工作", "通勤", "--json")
    assert code == 0, error
    assert json.loads(output)["name"] == "通勤"
    assert store.playlist("通勤") == [tracks[0]]
    code, _, error = await run_cli("playlist", "rename", "通勤", "备份", "--json")
    assert code == 1
    assert "同名" in json.loads(error)["error"]
    code, output, error = await run_cli("playlist", "delete", "通勤", "--json")
    assert code == 0, error
    assert json.loads(output)["deleted"] == "通勤"
    assert store.playlist("备份") == [tracks[0]]
    store.close()
