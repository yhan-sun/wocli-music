"""Serialized local control API and background playback lifecycle."""

import asyncio
import contextlib
import json
import logging
import os
import signal

from .config import MODES, QUALITIES, Paths, Settings
from .downloads import Downloads
from .lyrics import parse_lrc
from .models import IncompleteAudio, MusicError, NetworkError, Track
from .order import ShuffleOrder
from .player import Mpv
from .provider import KuwoProvider
from .store import Store

log = logging.getLogger(__name__)
LYRICS_TIMEOUT = 22
SOURCE_TIMEOUT = 45
RETRY_DELAY = 0.8


class MusicService:
    def __init__(self, paths, *, provider=None, player_factory=Mpv):
        self.paths = paths
        self.settings = Settings(paths)
        self.store = Store(paths)
        self.downloads = Downloads(self.store)
        self.provider = provider or KuwoProvider(direct=self.settings.values["direct"])
        self.player = player_factory(paths, self._player_event)
        self.player.direct = self.settings.values["direct"]
        saved = self.store.load("queue", {})
        self.queue = [Track.from_data(t) for t in saved.get("tracks", [])]
        self.index = min(max(saved.get("index", -1), -1), len(self.queue) - 1)
        self.shuffle = ShuffleOrder()
        self.shuffle.reset(len(self.queue), self.index)
        self.phase = "stopped"
        self.error = ""
        self.error_code = ""
        self.error_details = []
        self.actual_quality = ""
        self.generation = 0
        self.play_task = None
        self.lyric_task = None
        self.lyric_text = ""
        self.lyrics_status = "idle"
        self.lyrics_error = ""
        self.lyrics_generation = 0
        self.retry_attempt = 0
        self.requested_quality = self.settings.values["quality"]
        self.lock = asyncio.Lock()
        self.shutdown = asyncio.Event()
        self.entry_id = None
        self.event_tasks = set()

    @property
    def current(self):
        return self.queue[self.index] if 0 <= self.index < len(self.queue) else None

    def persist(self):
        self.store.save("queue", {"tracks": [t.data() for t in self.queue], "index": self.index})

    def status(self):
        current = self.current
        return {
            "downloads": self.downloads.status(),
            "retry_attempt": self.retry_attempt,
            "requested_quality": self.requested_quality,
            "phase": self.phase,
            "track": current.data() if current else None,
            "position": self.player.properties["time-pos"],
            "duration": self.player.properties["duration"] or (current.duration if current else 0),
            "volume": self.settings.values["volume"],
            "mode": self.settings.values["mode"],
            "quality": self.actual_quality,
            "preferred_quality": self.settings.values["quality"],
            "index": self.index,
            "queue": [t.data() for t in self.queue],
            "error": self.error,
            "error_code": self.error_code,
            "error_details": self.error_details,
            "liked": bool(current and self.store.is_favorite(current.id)),
            "lyrics": self.lyric_text,
            "lyrics_status": self.lyrics_status,
            "lyrics_error": self.lyrics_error,
            "lyric_offset": self.settings.values["lyric_offset"],
        }

    def _player_event(self, event):
        if event.get("event") == "start-file":
            self.entry_id = event.get("playlist_entry_id")
        if event.get("event") == "property-change" and event.get("name") == "pause":
            if self.phase in ("playing", "paused"):
                self.phase = "paused" if event.get("data") else "playing"
        if event.get("event") in ("end-file", "connection-lost"):
            task = asyncio.create_task(self._handle_end(event, self.generation))
            self.event_tasks.add(task)
            task.add_done_callback(self.event_tasks.discard)

    async def _handle_end(self, event, generation):
        async with self.lock:
            if generation != self.generation or self.phase not in ("playing", "paused"):
                return
            if event["event"] == "connection-lost":
                self.phase, self.error = "error", "播放器意外退出，请重新播放"
            elif event.get("playlist_entry_id", self.entry_id) != self.entry_id:
                return
            elif event.get("reason") == "eof":
                if self.settings.values["mode"] == "single":
                    self._begin(self.index)
                else:
                    await self._advance(1, automatic=True)
            elif event.get("reason") == "error":
                self.phase, self.error = "error", "音频读取中断，请重试或选择其他音质"

    def _cancel_loading(self, *, lyrics=True):
        self.generation += 1
        if lyrics:
            self.lyrics_generation += 1
        for task in (self.play_task, self.lyric_task if lyrics else None):
            if task and not task.done():
                task.cancel()

    def _begin(self, index, quality=None, *, advance=False, retry=False):
        if not 0 <= index < len(self.queue):
            raise MusicError("队列中没有这首歌曲")
        self._cancel_loading(lyrics=not retry)
        self.index = index
        if not advance:
            self.shuffle.reset(len(self.queue), index)
        self.phase, self.error, self.actual_quality = "loading", "", ""
        self.error_code, self.error_details = "", []
        self.retry_attempt = 0
        self.requested_quality = quality or self.settings.values["quality"]
        if not retry:
            self.lyric_text = self.store.lyric(self.current.id)
            if not parse_lrc(self.lyric_text):
                self.lyric_text = ""
            self.lyrics_error = ""
            self.lyrics_status = "ready" if self.lyric_text else "loading"
            if not self.lyric_text:
                self._start_lyrics()
        if retry and self.lyrics_status == "idle" and not self.lyric_text:
            self._start_lyrics()
        self.player.properties.update({"time-pos": 0, "duration": 0})
        self.persist()
        self.play_task = asyncio.create_task(
            self._load(self.current, quality or self.settings.values["quality"], self.generation)
        )

    async def _load(self, track, quality, generation):
        try:
            # Stop old audio before a potentially slow network lookup.
            if self.player.writer and not self.player.writer.is_closing():
                await self.player.command("stop")
            for attempt in range(2):
                self.retry_attempt = attempt + 1
                try:
                    async with asyncio.timeout(SOURCE_TIMEOUT):
                        stream = await asyncio.to_thread(self.provider.stream, track, quality)
                    break
                except (NetworkError, TimeoutError, ConnectionError):
                    if attempt == 1:
                        raise
                    await asyncio.sleep(RETRY_DELAY)
            if generation != self.generation:
                return
            if self.player.direct != self.settings.values["direct"]:
                await self.player.close()
                self.player.direct = self.settings.values["direct"]
            await self.player.load(stream.url, self.settings.values["volume"])
            if generation != self.generation:
                return
            actual_duration = self.player.properties["duration"]
            if track.duration > 30 and actual_duration and actual_duration < track.duration * 0.8:
                await self.player.command("stop")
                raise IncompleteAudio("音频时长明显短于歌曲信息，可能只有试听片段，已停止播放")
            self.actual_quality, self.phase = stream.quality, "playing"
            self.store.played(track)
        except asyncio.CancelledError:
            raise
        except Exception as error:
            if generation == self.generation:
                self.phase = "error"
                self.error = (
                    "音源请求超时，请重试"
                    if isinstance(error, TimeoutError)
                    else str(error)
                    if isinstance(error, MusicError)
                    else "播放失败，请查看 service.log"
                )
                self.error_code = getattr(error, "code", "playback_error")
                self.error_details = getattr(error, "details", [])
                if isinstance(error, MusicError):
                    log.warning(
                        "Could not load track %s: %s; details=%s",
                        track.id,
                        error,
                        self.error_details,
                    )
                else:
                    log.exception("Could not load track %s", track.id)

    def _start_lyrics(self):
        if self.lyric_task and not self.lyric_task.done():
            self.lyric_task.cancel()
        self.lyrics_generation += 1
        self.lyrics_status, self.lyrics_error = "loading", ""
        self.lyric_task = asyncio.create_task(self._lyrics(self.current, self.lyrics_generation))

    async def _lyrics(self, track, lyric_generation):
        try:
            async with asyncio.timeout(LYRICS_TIMEOUT):
                text = await asyncio.to_thread(self.provider.lyrics, track)
            if not isinstance(text, str) or not parse_lrc(text):
                text = ""
            if (
                lyric_generation == self.lyrics_generation
                and self.current
                and self.current.id == track.id
            ):
                if text:
                    self.lyric_text = text
                    self.lyrics_status = "ready"
                elif self.lyric_text:
                    self.lyrics_status = "ready"
                    self.lyrics_error = "未获取到新歌词，继续显示已有歌词。"
                else:
                    self.lyrics_status = "unavailable"
                if text:
                    self.store.lyric(track.id, text)
        except TimeoutError:
            if (
                lyric_generation == self.lyrics_generation
                and self.current
                and self.current.id == track.id
            ):
                self.lyrics_status = "ready" if self.lyric_text else "unavailable"
                self.lyrics_error = "歌词获取超时，按 l 重试。"
            log.warning("Lyrics timed out for %s", track.id)
        except Exception:
            if (
                lyric_generation == self.lyrics_generation
                and self.current
                and self.current.id == track.id
            ):
                self.lyrics_status = "ready" if self.lyric_text else "unavailable"
                self.lyrics_error = "歌词获取失败，按 l 重试。"
            log.warning("Lyrics unavailable for %s", track.id, exc_info=True)

    async def _advance(self, step, automatic=False):
        if not self.queue:
            raise MusicError("播放队列为空")
        mode = self.settings.values["mode"]
        if mode == "shuffle":
            index = (
                self.shuffle.previous()
                if step < 0
                else self.shuffle.next(len(self.queue), self.index)
            )
        else:
            index = self.index + step
        if not 0 <= index < len(self.queue):
            if mode in ("repeat", "single", "shuffle"):
                index %= len(self.queue)
            elif automatic:
                await self._stop()
                return
            else:
                raise MusicError("已经是队列的第一首或最后一首")
        self._begin(index, advance=True)

    async def _stop(self):
        self._cancel_loading()
        self.phase, self.error = "stopped", ""
        self.error_code, self.error_details = "", []
        if self.lyrics_status == "loading":
            self.lyrics_status = "idle"
        if self.player.writer and not self.player.writer.is_closing():
            await self.player.command("stop")
        self.player.properties.update({"time-pos": 0, "duration": 0})

    async def dispatch(self, request):
        operation = request.get("op")
        async with self.lock:
            if operation == "status":
                return self.status()
            if operation == "play":
                quality = request.get("quality")
                if quality is not None and quality not in QUALITIES:
                    raise MusicError("无效音质")
                tracks = [Track.from_data(t) for t in request.get("tracks", [])]
                index = int(request.get("index", 0 if tracks else max(0, self.index)))
                if not 0 <= index < len(tracks or self.queue):
                    raise MusicError("队列中没有这首歌曲")
                if tracks:
                    self.queue = tracks
                    self.store.remember(tracks)
                self._begin(index, quality)
            elif operation == "retry":
                if not self.current:
                    raise MusicError("请先选择一首歌")
                if self.phase == "loading":
                    return self.status()
                quality = request.get("quality", self.requested_quality)
                if quality not in QUALITIES:
                    raise MusicError("无效音质")
                self._begin(self.index, quality, advance=True, retry=True)
            elif operation == "lyrics_retry":
                if not self.current:
                    raise MusicError("请先播放一首歌")
                if self.lyrics_status != "loading":
                    self._start_lyrics()
            elif operation == "download":
                quality = request.get("quality", self.settings.values["quality"])
                if quality not in QUALITIES:
                    raise MusicError("无效音质")
                directory = request.get("directory")
                if not isinstance(directory, str) or not directory.strip():
                    raise MusicError("请选择下载目录")
                self.downloads.start(
                    self.provider,
                    Track.from_data(request["track"]),
                    directory,
                    quality,
                    bool(request.get("lyrics", True)),
                )
            elif operation == "download_cancel":
                self.downloads.cancel(request["id"])
            elif operation == "download_retry":
                self.downloads.retry(request["id"], self.provider)
            elif operation in ("pause", "resume", "toggle"):
                if self.phase not in ("playing", "paused"):
                    raise MusicError("当前没有正在播放的音乐")
                pause = operation == "pause" or (operation == "toggle" and self.phase == "playing")
                await self.player.command("set_property", "pause", pause)
                self.phase = "paused" if pause else "playing"
            elif operation in ("next", "prev"):
                await self._advance(1 if operation == "next" else -1)
            elif operation == "stop":
                await self._stop()
            elif operation == "seek":
                if self.phase not in ("playing", "paused"):
                    raise MusicError("当前没有正在播放的音乐")
                seconds = float(request["seconds"])
                if not -86400 <= seconds <= 86400:
                    raise MusicError("跳转时间超出范围")
                await self.player.command("seek", seconds, "relative+exact")
            elif operation == "volume":
                value = int(request["value"])
                self.settings.set("volume", value)
                if self.player.writer and not self.player.writer.is_closing():
                    await self.player.command("set_property", "volume", value)
            elif operation == "config":
                self.settings.set(request["key"], request["value"])
                if request["key"] == "direct":
                    self.provider = KuwoProvider(direct=request["value"])
                elif request["key"] == "mode":
                    self.shuffle.reset(len(self.queue), self.index)
                elif (
                    request["key"] == "volume"
                    and self.player.writer
                    and not self.player.writer.is_closing()
                ):
                    await self.player.command("set_property", "volume", request["value"])
            elif operation == "mode":
                value = request["value"]
                if value not in MODES:
                    raise MusicError("无效播放模式")
                self.settings.set("mode", value)
                self.shuffle.reset(len(self.queue), self.index)
            elif operation == "queue_add":
                tracks = [Track.from_data(t) for t in request["tracks"]]
                self.store.remember(tracks)
                position = self.index + 1 if request.get("next") else len(self.queue)
                self.queue[position:position] = tracks
                if position <= self.index:
                    self.index += len(tracks)
                self.shuffle.reset(len(self.queue), self.index)
                self.persist()
            elif operation == "queue_remove":
                index = int(request["index"])
                if not 0 <= index < len(self.queue):
                    raise MusicError("队列序号超出范围")
                if index == self.index:
                    await self._stop()
                self.queue.pop(index)
                if index <= self.index:
                    self.index -= 1
                self.shuffle.reset(len(self.queue), self.index)
                self.persist()
            elif operation == "queue_clear":
                await self._stop()
                self.queue, self.index = [], -1
                self.shuffle.reset(0, -1)
                self.persist()
            elif operation == "like":
                track = Track.from_data(request["track"]) if request.get("track") else self.current
                if not track:
                    raise MusicError("请先选择或播放一首歌")
                self.store.favorite(track)
            elif operation == "shutdown":
                self.shutdown.set()
            else:
                raise MusicError(f"未知操作：{operation}")
            return self.status()

    async def handle_client(self, reader, writer):
        try:
            raw = await asyncio.wait_for(reader.readline(), 10)
            request = json.loads(raw)
            if not isinstance(request, dict):
                raise ValueError("request must be an object")
            result = {"ok": True, "data": await self.dispatch(request)}
        except (TimeoutError, MusicError, ValueError, TypeError, KeyError) as error:
            result = {"ok": False, "error": str(error)}
        except Exception:
            log.exception("Control request failed")
            result = {"ok": False, "error": "服务请求失败，请查看 service.log"}
        try:
            writer.write((json.dumps(result, ensure_ascii=False) + "\n").encode())
            await writer.drain()
        except (ConnectionError, BrokenPipeError):
            pass
        finally:
            writer.close()
            with contextlib.suppress(OSError):
                await writer.wait_closed()

    async def close(self):
        self._cancel_loading()
        pending = [t for t in (self.play_task, self.lyric_task) if t]
        for task in self.event_tasks:
            task.cancel()
        await asyncio.gather(*pending, *self.event_tasks, return_exceptions=True)
        await self.downloads.close()
        await self.player.close()
        self.persist()
        self.store.close()


async def serve(paths=None):
    if os.name == "nt":
        raise MusicError("当前后台服务支持 macOS / Linux，Windows 支持尚在规划中")
    import fcntl

    paths = paths or Paths.discover()
    with (paths.runtime / "service.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return
        paths.socket.unlink(missing_ok=True)
        service = MusicService(paths)
        loop = asyncio.get_running_loop()
        for signum in (signal.SIGTERM, signal.SIGINT):
            loop.add_signal_handler(signum, service.shutdown.set)
        server = await asyncio.start_unix_server(
            service.handle_client, str(paths.socket), limit=2 * 1024 * 1024
        )
        paths.socket.chmod(0o600)
        try:
            async with server:
                await service.shutdown.wait()
        finally:
            server.close()
            await server.wait_closed()
            await service.close()
            paths.socket.unlink(missing_ok=True)


if __name__ == "__main__":
    paths = Paths.discover()
    logging.basicConfig(
        filename=paths.home / "service.log",
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
    )
    asyncio.run(serve(paths))
