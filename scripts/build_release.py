"""Build wheel/sdist release assets and their SHA-256 manifest using uv."""

import argparse
import ast
import hashlib
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="Use cached build dependencies")
    args = parser.parse_args()
    metadata = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf8"))
    version = metadata["project"]["version"]
    module = ast.parse((ROOT / "src/wocli_music/__init__.py").read_text(encoding="utf8"))
    package_version = next(
        ast.literal_eval(node.value)
        for node in module.body
        if isinstance(node, ast.Assign)
        and any(
            isinstance(target, ast.Name) and target.id == "__version__" for target in node.targets
        )
    )
    if version != package_version:
        raise SystemExit(f"Version mismatch: pyproject={version}, package={package_version}")
    command = ["uv", "build", "--out-dir", str(ROOT / "dist")]
    if args.offline:
        command.append("--offline")
    subprocess.run(command, cwd=ROOT, check=True)
    files = [
        ROOT / "dist" / f"wocli_music-{version}-py3-none-any.whl",
        ROOT / "dist" / f"wocli_music-{version}.tar.gz",
    ]
    lines = [f"{hashlib.sha256(file.read_bytes()).hexdigest()}  {file.name}\n" for file in files]
    manifest = ROOT / "dist/SHA256SUMS"
    manifest.write_text("".join(lines), encoding="utf8")
    for file in [*files, manifest]:
        print(f"{file.relative_to(ROOT)} ({file.stat().st_size:,} bytes)")


if __name__ == "__main__":
    main()
