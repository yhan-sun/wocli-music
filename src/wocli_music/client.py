import asyncio
import json
import os
import subprocess
import sys

from .config import Paths
from .models import MusicError


class Client:
    def __init__(self, paths=None):
        self.paths = paths or Paths.discover()

    async def request(self, op: str, *, autostart=True, **arguments):
        if os.name == "nt":
            raise MusicError("当前版本支持 macOS / Linux")
        try:
            reader, writer = await asyncio.open_unix_connection(
                str(self.paths.socket), limit=2 * 1024 * 1024
            )
        except (FileNotFoundError, ConnectionRefusedError):
            if not autostart:
                return None
            await self.start()
            reader, writer = await asyncio.open_unix_connection(
                str(self.paths.socket), limit=2 * 1024 * 1024
            )
        try:
            writer.write((json.dumps({"op": op, **arguments}, ensure_ascii=False) + "\n").encode())
            await writer.drain()
            line = await asyncio.wait_for(reader.readline(), 15)
            if not line:
                raise MusicError("后台服务已关闭，请重试")
            result = json.loads(line)
            if not result.get("ok"):
                raise MusicError(result.get("error", "后台服务请求失败"))
            return result["data"]
        except (TimeoutError, OSError, ValueError) as error:
            raise MusicError("后台服务连接中断或响应超时") from error
        finally:
            writer.close()
            await writer.wait_closed()

    async def start(self):
        # A service-level file lock resolves simultaneous cold starts safely.
        with (self.paths.home / "service.log").open("ab") as log:
            process = await asyncio.to_thread(
                subprocess.Popen,
                [sys.executable, "-m", "wocli_music.service"],
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=log,
                start_new_session=True,
                close_fds=True,
                env={**os.environ, "WOCLI_HOME": str(self.paths.home)},
            )
        for _ in range(100):
            try:
                _, writer = await asyncio.open_unix_connection(str(self.paths.socket))
                writer.close()
                await writer.wait_closed()
                return
            except (FileNotFoundError, ConnectionRefusedError):
                if process.poll() not in (None, 0):
                    raise MusicError(
                        f"后台服务启动失败，日志：{self.paths.home / 'service.log'}"
                    ) from None
                await asyncio.sleep(0.05)
        raise MusicError(f"后台服务启动超时，日志：{self.paths.home / 'service.log'}")

    async def wait_until_ready(self, deadline_seconds=90):
        async with asyncio.timeout(deadline_seconds):
            while True:
                state = await self.request("status")
                if state["phase"] != "loading":
                    if state["phase"] == "error":
                        raise MusicError(state["error"])
                    return state
                await asyncio.sleep(0.15)
