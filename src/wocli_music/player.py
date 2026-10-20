import asyncio
import json
import os

from .config import mpv_command
from .models import MusicError
from .provider import USER_AGENT


class Mpv:
    """One persistent mpv process, controlled exclusively through JSON IPC."""

    def __init__(self, paths, on_event):
        self.paths = paths
        self.on_event = on_event
        self.socket = paths.runtime / "mpv.sock"
        self.process = None
        self.writer = None
        self.reader_task = None
        self.pending = {}
        self.serial = 0
        self.loaded = None
        self.closing = False
        self.start_lock = asyncio.Lock()
        self.direct = False
        self.properties = {"pause": False, "time-pos": 0, "duration": 0, "volume": 60}

    async def start(self):
        async with self.start_lock:
            if self.writer and not self.writer.is_closing():
                return
            if self.process:
                await self.close()
            self.closing = False
            self.socket.unlink(missing_ok=True)
            arguments = [
                mpv_command(),
                "--idle=yes",
                "--no-video",
                "--no-terminal",
                "--no-config",
                "--audio-display=no",
                "--ytdl=no",
                f"--input-ipc-server={self.socket}",
                f"--user-agent={USER_AGENT}",
            ]
            if output := os.environ.get("WOCLI_AUDIO_OUTPUT"):
                arguments.append(f"--ao={output}")
            with (self.paths.home / "mpv.log").open("ab") as log:
                try:
                    self.process = await asyncio.create_subprocess_exec(
                        *arguments,
                        stdin=asyncio.subprocess.DEVNULL,
                        stdout=log,
                        stderr=log,
                        env={
                            key: value
                            for key, value in os.environ.items()
                            if not (
                                self.direct
                                and key.lower() in ("http_proxy", "https_proxy", "all_proxy")
                            )
                        },
                    )
                except OSError as error:
                    raise MusicError("无法启动 mpv，请运行 wocli doctor 检查安装") from error
            for _ in range(100):
                if self.process.returncode is not None:
                    raise MusicError(f"mpv 启动失败，日志：{self.paths.home / 'mpv.log'}")
                try:
                    reader, self.writer = await asyncio.open_unix_connection(str(self.socket))
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    await asyncio.sleep(0.05)
            else:
                await self.close()
                raise MusicError("连接 mpv 超时")
            self.reader_task = asyncio.create_task(self._read(reader))
            for index, name in enumerate(self.properties):
                await self.command("observe_property", index, name)

    async def _read(self, reader):
        try:
            while line := await reader.readline():
                try:
                    message = json.loads(line)
                except ValueError:
                    continue
                request_id = message.get("request_id")
                if request_id is not None:
                    future = self.pending.get(request_id)
                    if future and not future.done():
                        if message.get("error", "success") != "success":
                            future.set_exception(MusicError(f"mpv：{message['error']}"))
                        else:
                            future.set_result(message.get("data"))
                if message.get("event") == "property-change":
                    name = message.get("name")
                    if name in self.properties and message.get("data") is not None:
                        self.properties[name] = message["data"]
                if message.get("event") == "file-loaded" and self.loaded and not self.loaded.done():
                    self.loaded.set_result(True)
                if (
                    message.get("event") == "end-file"
                    and message.get("reason") == "error"
                    and self.loaded
                    and not self.loaded.done()
                ):
                    self.loaded.set_exception(MusicError("mpv 无法读取该音频"))
                self.on_event(message)
        except (OSError, asyncio.LimitOverrunError):
            pass
        finally:
            error = MusicError("mpv 连接已关闭，请重新播放")
            for future in list(self.pending.values()):
                if not future.done():
                    future.set_exception(error)
            if self.loaded and not self.loaded.done():
                self.loaded.set_exception(error)
            if self.writer:
                self.writer.close()
            if not self.closing:
                self.on_event({"event": "connection-lost"})

    async def command(self, *arguments):
        if not self.writer or self.writer.is_closing():
            raise MusicError("播放器尚未启动")
        self.serial += 1
        request_id = self.serial
        future = asyncio.get_running_loop().create_future()
        self.pending[request_id] = future
        try:
            self.writer.write(
                (json.dumps({"command": arguments, "request_id": request_id}) + "\n").encode()
            )
            await self.writer.drain()
            return await asyncio.wait_for(future, 5)
        except (TimeoutError, OSError) as error:
            raise MusicError("播放器控制超时或连接中断") from error
        finally:
            self.pending.pop(request_id, None)

    async def load(self, url: str, volume: int):
        await self.start()
        await self.command("stop")
        self.properties.update({"time-pos": 0, "duration": 0, "pause": False})
        self.loaded = asyncio.get_running_loop().create_future()
        try:
            await self.command("set_property", "volume", volume)
            await self.command("loadfile", url, "replace")
            await self.command("set_property", "pause", False)
            await asyncio.wait_for(self.loaded, 25)
        except TimeoutError as error:
            raise MusicError("音频加载超时，请重试或降低音质") from error
        finally:
            self.loaded = None

    async def close(self):
        self.closing = True
        if self.writer:
            try:
                await self.command("quit")
            except MusicError:
                pass
            self.writer.close()
            try:
                await self.writer.wait_closed()
            except OSError:
                pass
            self.writer = None
        if self.process and self.process.returncode is None:
            try:
                await asyncio.wait_for(self.process.wait(), 2)
            except TimeoutError:
                self.process.terminate()
                await self.process.wait()
        if self.reader_task:
            self.reader_task.cancel()
            await asyncio.gather(self.reader_task, return_exceptions=True)
            self.reader_task = None
        self.process = None
        self.socket.unlink(missing_ok=True)
