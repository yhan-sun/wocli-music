import hashlib
import html
import re
from dataclasses import asdict, dataclass
from pathlib import Path


class MusicError(Exception):
    """An actionable error safe to display in the terminal."""


class NetworkError(MusicError):
    """A transient source request failure that may be retried."""


class AudioUnavailable(MusicError):
    code = "source_unavailable"

    def __init__(self, message: str, details=()):
        super().__init__(message)
        self.details = list(details)


class IncompleteAudio(AudioUnavailable):
    code = "incomplete_audio"


def clean(value: object) -> str:
    text = re.sub(r"\\\\?u([0-9a-fA-F]{4})", lambda m: chr(int(m[1], 16)), str(value or ""))
    text = re.sub(r"<[^>]*>", "", html.unescape(text))
    text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)
    text = re.sub(r"[\x00-\x1f\x7f-\x9f]", " ", text)
    return " ".join(text.split())


def normalize_id(value: str) -> str:
    value = re.sub(r"^(?:kuwo:|MUSIC_)", "", value.strip(), flags=re.I)
    if not value.isascii() or not value.isdigit():
        raise MusicError("歌曲 ID 应为数字，例如 kuwo:228908")
    return f"kuwo:{int(value)}"


def clock(seconds: float | int | None) -> str:
    value = max(0, int(seconds or 0))
    return f"{value // 60:02d}:{value % 60:02d}"


@dataclass
class Track:
    id: str
    title: str
    artist: str = "未知歌手"
    album: str = ""
    duration: float = 0
    formats: str = ""
    path: str = ""

    def data(self) -> dict:
        return asdict(self)

    @classmethod
    def from_data(cls, data: dict) -> "Track":
        return cls(**{key: data[key] for key in cls.__dataclass_fields__ if key in data})

    @classmethod
    def local(cls, path: str) -> "Track":
        file = Path(path).expanduser().resolve()
        if not file.is_file():
            raise MusicError(f"找不到音频文件：{file}")
        identity = hashlib.sha256(str(file).encode()).hexdigest()[:20]
        return cls(f"local:{identity}", clean(file.stem), "本地音乐", path=str(file))


@dataclass
class Stream:
    url: str
    quality: str
    size: int
    extension: str
