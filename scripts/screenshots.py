"""Capture the real TUI with isolated demo data; no network or audio output.

uv run --with resvg-py python scripts/screenshots.py --format both
"""

import argparse
import asyncio
import os
import sys
import tempfile
from pathlib import Path

from preview import terminal_svg
from textual.widgets import DataTable

from wocli_music.config import Paths
from wocli_music.models import Track
from wocli_music.tui import MusicApp

LYRICS = """[00:00]城市的灯，一盏盏亮起
[00:15]晚风翻过没读完的书
[00:30]穿过街角，走过长桥
[00:45]远处的列车驶向星空
[01:00]让脚步慢一些
[01:15]让旋律再停留一会
[01:30]把一整天的喧闹，留在身后
[01:45]今晚的路还有很长
[02:00]沿着灯火，慢慢回家
[02:15]一首歌，陪你走到天亮"""

TRACKS = [
    Track("demo:1", "夜航", "回声计划", "城市的另一面", 254),
    Track("demo:2", "雨后的街道", "林间", "慢慢听", 218),
    Track("demo:3", "山海之间", "远山", "山海之间", 286),
    Track("demo:4", "Sunday Morning", "Paper Moon", "Small Things", 231),
    Track("demo:5", "散步到月亮升起", "回声计划", "城市的另一面", 265),
    Track("demo:6", "光の向こう", "青い部屋", "夜明け", 243),
    Track("demo:7", "末班车", "林间", "慢慢听", 207),
    Track("demo:8", "A Quiet Place", "Paper Moon", "Small Things", 312),
]


class PreviewClient:
    def __init__(self):
        self.state = {
            "track": TRACKS[0].data(),
            "queue": [track.data() for track in TRACKS],
            "index": 0,
            "phase": "playing",
            "position": 93,
            "duration": TRACKS[0].duration,
            "liked": True,
            "mode": "repeat",
            "volume": 60,
            "quality": "320k",
            "error": "",
            "error_code": "",
            "error_details": [],
            "lyrics": LYRICS,
            "lyrics_status": "ready",
            "lyric_offset": 0,
            "downloads": [],
        }

    async def request(self, operation, **kwargs):
        if operation != "status":
            raise RuntimeError("Screenshot clients only support read-only status polling")
        return self.state.copy()


def export(app, output, name, args):
    svg = terminal_svg(app.export_screenshot())
    if args.format in ("svg", "both"):
        target = output / f"{name}.svg"
        target.write_text(svg, encoding="utf8")
        print(target.resolve())
    if args.format in ("png", "both"):
        try:
            import resvg_py
        except ImportError as error:
            raise SystemExit(
                "PNG export requires: uv run --with resvg-py python scripts/screenshots.py"
            ) from error
        target = output / f"{name}.png"
        target.write_bytes(resvg_py.svg_to_bytes(svg_string=svg, monospace_family=args.font))
        print(target.resolve())


async def capture(args, size, name, *, collection=False):
    with tempfile.TemporaryDirectory(prefix="wocli-screenshots-") as temporary:
        root = Path(temporary)
        paths = Paths(root / "data", root / "run")
        paths.home.mkdir()
        paths.runtime.mkdir()
        app = MusicApp(paths=paths)
        app.client = PreviewClient()
        for track in TRACKS:
            app.store.add_to_playlist("深夜电台", track)
        for track in TRACKS[1:4]:
            app.store.add_to_playlist("通勤路上", track)
        app.store.create_playlist("慢慢听")
        app.store.favorite(TRACKS[0])
        app.store.favorite(TRACKS[2])
        app.store.save("ui.last_folder", "慢慢听")
        app.active_view = "playlist:深夜电台"
        async with app.run_test(size=size) as pilot:
            await pilot.pause()
            app.query_one("#results", DataTable).focus()
            app.render_state(app.client.state)
            await pilot.pause()
            export(app, args.output, name, args)
            if collection:
                await pilot.click("#collect")
                await pilot.pause()
                await pilot.click("#folder-select", offset=(8, 1))
                await pilot.pause()
                export(app, args.output, "collection", args)


async def main(args):
    os.environ.pop("NO_COLOR", None)
    args.output.mkdir(parents=True, exist_ok=True)
    await capture(args, (123, 36), "library", collection=True)
    await capture(args, (80, 26), "compact")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("docs/screenshots"))
    parser.add_argument("--format", choices=("svg", "png", "both"), default="svg")
    parser.add_argument(
        "--font", default="Menlo" if sys.platform == "darwin" else "DejaVu Sans Mono"
    )
    asyncio.run(main(parser.parse_args()))
