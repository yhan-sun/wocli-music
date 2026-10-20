import math
import re
import unicodedata
from bisect import bisect_right
from dataclasses import dataclass

from .models import clean


def lyric_title(title: str) -> str:
    """Remove annotation-only suffixes while retaining live/remix/version identity."""
    title = unicodedata.normalize("NFKC", clean(title))
    annotation = r"\((?:カバー|cover(?:\s*[:：].*?)?|翻唱|explicit|clean)(?:\s+version)?\)"
    return re.sub(r"\s+", " ", re.sub(annotation, "", title, flags=re.I)).strip()


def lyric_matches(track, title: str, artists: list[str], duration: float) -> bool:
    def normalized(value):
        return re.sub(r"\s+", "", unicodedata.normalize("NFKC", value)).casefold()

    if normalized(lyric_title(title)) != normalized(lyric_title(track.title)):
        return False
    expected_artists = {
        normalized(artist) for artist in re.split(r"[&/、,，]", track.artist) if artist
    }
    if not expected_artists.intersection(normalized(artist) for artist in artists if artist):
        return False
    return not track.duration or bool(duration and abs(duration - track.duration) <= 8)


def lines_to_lrc(entries: list) -> str:
    lines = []
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("lineLyric"), str):
            continue
        try:
            seconds = float(entry["time"])
            if not math.isfinite(seconds) or seconds < 0:
                continue
            centiseconds = round(seconds * 100)
        except (ValueError, TypeError, KeyError):
            continue
        minutes, remainder = divmod(centiseconds, 6000)
        second, fraction = divmod(remainder, 100)
        lines.append(f"[{minutes:02d}:{second:02d}.{fraction:02d}]{clean(entry['lineLyric'])}")
    return "\n".join(lines)


@dataclass(frozen=True)
class Line:
    time: float
    text: str


def parse_lrc(text: str) -> list[Line]:
    offset_match = re.search(r"\[offset:([+-]?\d+)\]", text, re.I)
    offset = int(offset_match[1]) / 1000 if offset_match else 0
    lines = []
    for raw in text.splitlines():
        stamps = list(re.finditer(r"\[(\d+):(\d{2})(?:[.:](\d{1,3}))?\]", raw))
        if not stamps:
            continue
        words = clean(raw[stamps[-1].end() :])
        for stamp in stamps:
            fraction = float("0." + stamp[3]) if stamp[3] else 0
            lines.append(
                Line(max(0, int(stamp[1]) * 60 + int(stamp[2]) + fraction + offset), words)
            )
    return sorted(lines, key=lambda line: line.time)


def current_line(lines: list[Line], position: float, offset_ms: int = 0) -> int:
    return bisect_right([line.time for line in lines], position + offset_ms / 1000) - 1
