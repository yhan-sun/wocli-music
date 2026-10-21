import hashlib
import json
import os
import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path

from .models import MusicError
from .runtime import bundled_mpv

DEFAULTS = {"quality": "320k", "volume": 60, "mode": "list", "direct": False, "lyric_offset": 0}
QUALITIES = ("best", "flac", "320k", "128k")
MODES = ("list", "repeat", "single", "shuffle")


@dataclass(frozen=True)
class Paths:
    home: Path
    runtime: Path

    @classmethod
    def discover(cls) -> "Paths":
        config_root = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
        home = (
            Path(os.environ.get("WOCLI_HOME", config_root / "wocli-music")).expanduser().resolve()
        )
        tag = hashlib.sha256(str(home).encode()).hexdigest()[:12]
        uid = os.getuid() if hasattr(os, "getuid") else "user"
        runtime = Path(tempfile.gettempdir()) / f"wocli-{uid}-{tag}"
        for directory in (home, runtime):
            directory.mkdir(parents=True, exist_ok=True, mode=0o700)
            directory.chmod(0o700)
        return cls(home, runtime)

    @property
    def socket(self) -> Path:
        return self.runtime / "control.sock"

    @property
    def database(self) -> Path:
        return self.home / "library.sqlite3"


class Settings:
    def __init__(self, paths: Paths):
        self.file = paths.home / "settings.json"
        self.values = dict(DEFAULTS)
        if self.file.exists():
            try:
                values = json.loads(self.file.read_text())
                if not isinstance(values, dict):
                    raise ValueError("配置必须是 JSON 对象")
                for key, value in values.items():
                    if key in DEFAULTS:
                        self._validate(key, value)
                        self.values[key] = value
            except (ValueError, TypeError) as error:
                raise MusicError(f"配置文件无效：{self.file} ({error})") from error

    @staticmethod
    def _validate(key: str, value):
        if key not in DEFAULTS:
            raise MusicError(f"未知配置项：{key}")
        if key == "quality" and value not in QUALITIES:
            raise MusicError(f"音质可选：{', '.join(QUALITIES)}")
        if key == "mode" and value not in MODES:
            raise MusicError(f"播放模式可选：{', '.join(MODES)}")
        if key == "volume" and (type(value) is not int or not 0 <= value <= 100):
            raise MusicError("音量应为 0 到 100 的整数")
        if key == "direct" and type(value) is not bool:
            raise MusicError("direct 应为 true 或 false")
        if key == "lyric_offset" and (type(value) is not int or abs(value) > 60000):
            raise MusicError("歌词偏移应为 -60000 到 60000 毫秒")

    def set(self, key, value):
        self._validate(key, value)
        self.values[key] = value
        temporary = self.file.with_suffix(".tmp")
        temporary.write_text(json.dumps(self.values, ensure_ascii=False, indent=2) + "\n")
        temporary.replace(self.file)


def mpv_command() -> str:
    bundled = bundled_mpv()
    executable = (
        os.environ.get("WOCLI_MPV")
        or (str(bundled) if bundled and bundled.is_file() and os.access(bundled, os.X_OK) else None)
        or shutil.which("mpv")
    )
    if not executable:
        candidates = [
            Paths.discover().home / "tools/mpv.app/Contents/MacOS/mpv",
            Path.home() / "Applications/mpv.app/Contents/MacOS/mpv",
            Path("/Applications/mpv.app/Contents/MacOS/mpv"),
        ]
        executable = next(
            (str(path) for path in candidates if path.is_file() and os.access(path, os.X_OK)), None
        )
    if not executable:
        raise MusicError("未找到 mpv。macOS：brew install mpv；Ubuntu：sudo apt install mpv")
    return executable
