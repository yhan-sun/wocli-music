"""Export the real Rust TUI with isolated demo data; no network or audio.

uv run --with resvg-py python scripts/screenshots_native.py
"""
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
output = root / "docs/screenshots"
subprocess.run(["cargo", "test", "tui::screenshots::export", "--", "--ignored"], cwd=root, env={**os.environ, "WOCLI_SCREENSHOTS": str(output)}, check=True)
import resvg_py  # build-time rendering only

for source in [output / f"{name}.svg" for name in ("library", "compact", "collections", "boss", "playlist-import")]:
    source.with_suffix(".png").write_bytes(resvg_py.svg_to_bytes(svg_string=source.read_text(), monospace_family="Menlo"))
    print(source.with_suffix(".png"))
