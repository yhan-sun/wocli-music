"""Export a real search screen; correct Rich SVG widths for East Asian characters."""

import argparse
import asyncio
import html
import os
import re
from pathlib import Path

from rich.cells import cell_len
from textual.widgets import Input

from wocli_music.tui import MusicApp


def terminal_svg(svg: str) -> str:
    def repair(match):
        text = html.unescape(match[4])
        width = float(match[2])
        if text and cell_len(text) > len(text):
            width *= cell_len(text) / len(text)
        return f"<text{match[1]}{width:g}{match[3]}{match[4]}</text>"

    return re.sub(
        r'<text([^>]*textLength=")([\d.]+)("[^>]*>)(.*?)</text>', repair, svg, flags=re.DOTALL
    )


async def preview(args):
    # Preview exports use the actual palette even in a non-color CI environment.
    os.environ.pop("NO_COLOR", None)
    app = MusicApp()
    async with app.run_test(size=(args.width, args.height)) as pilot:
        app.query_one("#search", Input).value = args.query
        app.start_search()
        await app.workers.wait_for_complete()
        await pilot.pause()
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(terminal_svg(app.export_screenshot()), encoding="utf8")
    print(args.output.resolve())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--query", default="周杰伦 晴天")
    parser.add_argument("--width", type=int, default=120)
    parser.add_argument("--height", type=int, default=36)
    parser.add_argument("--output", type=Path, default=Path("artifacts/wocli-search.svg"))
    asyncio.run(preview(parser.parse_args()))
