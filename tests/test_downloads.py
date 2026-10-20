import asyncio
import threading
from pathlib import Path

from conftest import FakeProvider

from wocli_music.downloads import Downloads
from wocli_music.models import MusicError
from wocli_music.store import Store


class DownloadProvider(FakeProvider):
    def __init__(self):
        self.calls = 0
        self.fail = False

    def download(self, stream, target, *, cancelled, progress):
        self.calls += 1
        if self.fail:
            raise MusicError("下载网络中断")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(b"audio")
        progress(5, 5)


async def test_download_runs_in_background_deduplicates_and_keeps_existing_file(
    paths, tracks, tmp_path
):
    store = Store(paths)
    downloads = Downloads(store)
    provider = DownloadProvider()
    try:
        identity = downloads.start(provider, tracks[0], str(tmp_path), "320k")
        assert downloads.start(provider, tracks[0], str(tmp_path), "320k") == identity
        await downloads.tasks[identity]
        job = downloads.jobs[identity]
        assert job["phase"] == "done"
        target = Path(job["path"])
        assert await asyncio.to_thread(target.read_bytes) == b"audio"
        assert target.with_suffix(".lrc").read_text().startswith("[00:00.00]")
        assert provider.calls == 1
        second = downloads.start(provider, tracks[0], str(tmp_path), "320k")
        await downloads.tasks[second]
        assert downloads.jobs[second]["phase"] == "done"
        assert "已存在" in downloads.jobs[second]["warning"]
        assert provider.calls == 1
        third = downloads.start(provider, tracks[0], str(tmp_path), "128k")
        await downloads.tasks[third]
        assert downloads.jobs[third]["path"] != str(target)
        assert "[128k]" in downloads.jobs[third]["path"]
        assert provider.calls == 2
    finally:
        await downloads.close()
        store.close()


async def test_download_failure_retry_and_lyric_failure_do_not_lose_audio(paths, tracks, tmp_path):
    class BrokenLyrics(DownloadProvider):
        def lyrics(self, track):
            raise AttributeError("schema changed")

    store = Store(paths)
    downloads = Downloads(store)
    provider = BrokenLyrics()
    provider.fail = True
    try:
        identity = downloads.start(provider, tracks[0], str(tmp_path), "best")
        await downloads.tasks[identity]
        assert downloads.jobs[identity]["phase"] == "error"
        provider.fail = False
        retried = downloads.retry(identity, provider)
        await downloads.tasks[retried]
        job = downloads.jobs[retried]
        assert job["phase"] == "done"
        assert "歌词保存失败" in job["warning"]
        assert await asyncio.to_thread(Path(job["path"]).read_bytes) == b"audio"
    finally:
        await downloads.close()
        store.close()


async def test_download_cancel_during_resolution_discards_late_result(paths, tracks, tmp_path):
    entered, release = threading.Event(), threading.Event()

    class SlowProvider(DownloadProvider):
        def stream(self, track, quality):
            entered.set()
            release.wait(2)
            return super().stream(track, quality)

    store = Store(paths)
    downloads = Downloads(store)
    provider = SlowProvider()
    try:
        identity = downloads.start(provider, tracks[0], str(tmp_path), "best")
        await asyncio.to_thread(entered.wait, 1)
        downloads.cancel(identity)
        release.set()
        await downloads.tasks[identity]
        await asyncio.sleep(0.01)
        assert downloads.jobs[identity]["phase"] == "cancelled"
        assert provider.calls == 0
        assert not list(tmp_path.glob("*.mp3"))
    finally:
        release.set()
        await downloads.close()
        store.close()
