import asyncio

import pytest

from wocli_music.config import Paths
from wocli_music.models import MusicError, Stream, Track


@pytest.fixture
def paths(tmp_path, monkeypatch):
    monkeypatch.setenv("WOCLI_HOME", str(tmp_path / "home"))
    return Paths.discover()


@pytest.fixture
def tracks():
    return [
        Track("kuwo:1", "晴天", "周杰伦", duration=269),
        Track("kuwo:2", "夜曲", "周杰伦", duration=226),
    ]


class FakeProvider:
    def stream(self, track, quality):
        if track.id == "kuwo:404":
            raise MusicError("音源暂不可用")
        return Stream(track.id, quality, 100, ".mp3")

    def lyrics(self, track):
        return "[00:00.00]前奏\n[00:01.00]歌词"


class FakeWriter:
    def is_closing(self):
        return False


class FakePlayer:
    def __init__(self, paths, callback):
        self.callback = callback
        self.properties = {"time-pos": 0, "duration": 0, "pause": False, "volume": 60}
        self.writer = None
        self.loads = []
        self.commands = []

    async def load(self, url, volume):
        self.loads.append(url)
        self.writer = FakeWriter()
        await asyncio.sleep(0)
        self.properties["duration"] = 269

    async def command(self, *args):
        self.commands.append(args)

    async def close(self):
        self.writer = None


@pytest.fixture
async def service(paths):
    from wocli_music.service import MusicService

    instance = MusicService(paths, provider=FakeProvider(), player_factory=FakePlayer)
    yield instance
    await instance.close()


async def loaded(service):
    if service.play_task:
        await service.play_task
    await asyncio.sleep(0)
