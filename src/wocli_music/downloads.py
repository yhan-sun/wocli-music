"""Background downloads, independent of the lifetime of a terminal interface."""

import asyncio
import re
import threading
import uuid
from pathlib import Path

from .lyrics import parse_lrc
from .models import MusicError, Track, clean


class Downloads:
    def __init__(self, store):
        self.store = store
        self.jobs = {}
        self.tasks = {}
        self.cancelled = {}
        self.slot = asyncio.Semaphore(1)

    def status(self):
        return [dict(job) for job in self.jobs.values()]

    def start(self, provider, track, directory, quality, lyrics=True):
        if track.path:
            raise MusicError("这首歌已在本地，无需下载")
        directory = str(Path(directory).expanduser().resolve())
        for job in self.jobs.values():
            if (job["track"]["id"], job["directory"], job["quality"]) == (
                track.id,
                directory,
                quality,
            ) and job["phase"] in ("queued", "resolving", "downloading", "lyrics"):
                return job["id"]
        if sum(not task.done() for task in self.tasks.values()) >= 10:
            raise MusicError("下载队列已满，请等待或取消部分任务")
        # Keep recent results, retaining every active job.
        for identity in list(self.jobs):
            if len(self.jobs) < 20:
                break
            if self.tasks[identity].done():
                self.jobs.pop(identity)
                self.tasks.pop(identity)
                self.cancelled.pop(identity)
        identity = uuid.uuid4().hex[:12]
        self.jobs[identity] = dict(
            id=identity,
            track=track.data(),
            directory=directory,
            quality=quality,
            lyrics=lyrics,
            phase="queued",
            received=0,
            total=0,
            path="",
            error="",
            warning="",
        )
        self.cancelled[identity] = threading.Event()
        self.tasks[identity] = asyncio.create_task(self._run(identity, provider))
        return identity

    def cancel(self, identity):
        if identity not in self.jobs:
            raise MusicError("找不到下载任务")
        if not self.tasks[identity].done():
            self.cancelled[identity].set()
            self.tasks[identity].cancel()
            self.jobs[identity]["phase"] = "cancelled"

    def retry(self, identity, provider):
        job = self.jobs.get(identity)
        if not job:
            raise MusicError("找不到下载任务")
        if job["phase"] not in ("error", "cancelled"):
            raise MusicError("该任务无需重试")
        return self.start(
            provider, Track.from_data(job["track"]), job["directory"], job["quality"], job["lyrics"]
        )

    async def _run(self, identity, provider):
        job, cancelled = self.jobs[identity], self.cancelled[identity]
        track = Track.from_data(job["track"])
        loop = asyncio.get_running_loop()

        def progress(done, total):
            def update():
                if not cancelled.is_set():
                    job.update(received=done, total=total)

            if not cancelled.is_set():
                try:
                    loop.call_soon_threadsafe(update)
                except RuntimeError:
                    pass

        try:
            async with self.slot:
                job["phase"] = "resolving"
                async with asyncio.timeout(60):
                    stream = await asyncio.to_thread(provider.stream, track, job["quality"])
                if cancelled.is_set():
                    return
                name = re.sub(r'[\\/:*?"<>|]', "_", clean(f"{track.artist} - {track.title}"))[
                    :65
                ].strip(". ")
                target = (
                    Path(job["directory"])
                    / f"{name} [{re.sub(r'[^a-zA-Z0-9_-]', '_', track.id.split(':')[-1])}] [{stream.quality}]{stream.extension}"
                )
                job.update(
                    phase="downloading",
                    total=stream.size,
                    path=str(target),
                    actual_quality=stream.quality,
                )
                if target.exists():
                    job["warning"] = "文件已存在，保留原文件"
                    job["received"] = target.stat().st_size
                else:
                    await asyncio.to_thread(
                        provider.download, stream, target, cancelled=cancelled, progress=progress
                    )
                if cancelled.is_set():
                    return
                if job["lyrics"]:
                    job["phase"] = "lyrics"
                    try:
                        text = self.store.lyric(track.id)
                        if not parse_lrc(text):
                            async with asyncio.timeout(22):
                                text = await asyncio.to_thread(provider.lyrics, track)
                        if parse_lrc(text):
                            sidecar = target.with_suffix(".lrc")
                            try:
                                with sidecar.open("x", encoding="utf8") as output:
                                    output.write(text)
                            except FileExistsError:
                                pass
                        else:
                            job["warning"] = "音频已保存，暂未找到歌词"
                    except Exception:
                        job["warning"] = "音频已保存，歌词保存失败"
                job["phase"] = "done"
        except asyncio.CancelledError:
            cancelled.set()
            job["phase"] = "cancelled"
        except Exception as error:
            job.update(
                phase="error",
                error=str(error)
                if isinstance(error, (MusicError, OSError))
                else "下载失败，请重试",
            )

    async def close(self):
        for identity in list(self.jobs):
            self.cancel(identity)
        await asyncio.gather(*self.tasks.values(), return_exceptions=True)
