"""Package the native executable and dependency notices (build-time Python only)."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile


def archive(source, target):
    with target.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=9, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as output:
            output.add(source, arcname=source.name)
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    target.with_suffix(target.suffix + ".sha256").write_text(f"{digest}  {target.name}\n")
    print(f"{target}: {target.stat().st_size:,} bytes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--alsa-source", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"]))
    own = next(p for p in metadata["packages"] if p["name"] == "wocli-music")
    version = own["version"]
    name = f"wocli-music-{version}-{args.platform}"
    staging = root / "build" / "native"
    staging.mkdir(parents=True, exist_ok=True)
    bundle = staging / name
    if bundle.exists():
        shutil.rmtree(bundle)
    bundle.mkdir()
    shutil.copy2(root / "target/release/wocli", bundle / "wocli")
    sources = staging / (name + "-sources")
    if sources.exists():
        shutil.rmtree(sources)
    sources.mkdir()
    notices = ["wocli-music native dependencies", "", "These notices apply to third-party code, not the project's own license.", "MPL sources and (on Linux) ALSA sources are provided in the matching -sources archive.", ""]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["name"] == "wocli-music":
            continue
        label = f'{package["name"]}-{package["version"]}'
        source = Path(package["manifest_path"]).parent
        target = bundle / "licenses" / label
        for document in source.rglob("*"):
            if document.is_file() and document.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")):
                destination = target / document.relative_to(source)
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(document, destination)
        license_name = package.get("license") or "see package license files"
        notices.append(f'{label}: {license_name}; {package.get("repository") or "https://crates.io/crates/" + package["name"]}')
        if "MPL" in license_name or "LGPL" in license_name:
            shutil.copytree(source, sources / label, ignore=shutil.ignore_patterns(".cargo-ok", ".cargo_vcs_info.json"))
    if args.platform.startswith("linux"):
        library = Path("/usr/lib/x86_64-linux-gnu/libasound.so.2").resolve()
        (bundle / "lib").mkdir()
        shutil.copy2(library, bundle / "lib/libasound.so.2")
        subprocess.run(["patchelf", "--set-rpath", "$ORIGIN/lib", str(bundle / "wocli")], check=True)
        (bundle / "licenses/alsa").mkdir()
        shutil.copy2("/usr/share/doc/libasound2/copyright", bundle / "licenses/alsa/copyright")
        notices.append("ALSA libasound.so.2: LGPL-2.1-or-later; https://www.alsa-project.org/")
        if args.alsa_source is None:
            raise SystemExit("Linux packaging requires --alsa-source directory with the matching Debian source files")
        shutil.copytree(args.alsa_source, sources / "alsa")
    (bundle / "THIRD_PARTY.txt").write_text("\n".join(notices) + "\n")
    (bundle / "README.txt").write_text(f"wocli-music {version}\n\nRun ./wocli. No Python, mpv or FFmpeg required.\nmacOS: wocli can be copied to ~/.local/bin.\nLinux: keep lib/ next to wocli; uses the desktop's audio configuration.\n\nDocumentation: https://github.com/yhan-sun/wocli-music\nData: ~/.config/wocli-music (or WOCLI_HOME / XDG_CONFIG_HOME).\nUpgrade from Python: stop the old daemon first; the library database is reused.\n\nThird-party notices: THIRD_PARTY.txt and licenses/.\nThe separate -sources archive is optional for running the player.\n")
    (sources / "README.txt").write_text("Source code of bundled MPL dependencies and, on Linux, the matching ALSA source package. Build tools: Rust/Cargo and a C compiler. Application source and Cargo.lock are available at the release tag in the repository.\n")
    dist = root / "dist"
    dist.mkdir(exist_ok=True)
    archive(bundle, dist / (name + ".tar.gz"))
    archive(sources, dist / (name + "-sources.tar.gz"))


if __name__ == "__main__":
    main()
