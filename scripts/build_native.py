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
import zipfile


def archive(source, target):
    if target.name.endswith(".zip"):
        with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as zf:
            for file_path in sorted(source.rglob("*")):
                if file_path.is_file():
                    arcname = f"{source.name}/{file_path.relative_to(source)}"
                    zf.write(file_path, arcname)
    else:
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
    is_windows = args.platform.startswith("windows")
    exe_name = "wocli.exe" if is_windows else "wocli"
    src_bin = root / "target/release" / exe_name
    if not src_bin.exists():
        src_bin = root / "target/release/wocli"
    shutil.copy2(src_bin, bundle / exe_name)
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
    if is_windows:
        reference_license = bundle / "licenses/TaskbarLyrics/LICENSE"
        reference_license.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / "docs/licenses/TaskbarLyrics-MIT.txt", reference_license)
        notices.append("TaskbarLyrics taskbar-hosting/occupied-space design reference: MIT; https://github.com/ANYNC/TaskbarLyrics (71262b42aaf0879d2a805b424107a95fbfd50e5a)")
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
    (bundle / "THIRD_PARTY.txt").write_text("\n".join(notices) + "\n", encoding="utf-8")
    (bundle / "README.txt").write_text(f"wocli-music {version}\n\nRun ./{exe_name}. No Python, mpv or FFmpeg required.\nmacOS: wocli can be copied to ~/.local/bin.\nLinux: keep lib/ next to wocli; uses the desktop's audio configuration.\nWindows: extract and run wocli.exe.\n\nDocumentation: https://github.com/yhan-sun/wocli-music\nData: ~/.config/wocli-music (or %APPDATA%\\wocli-music).\nUpgrade from Python: stop the old daemon first; the library database is reused.\n\nThird-party notices: THIRD_PARTY.txt and licenses/.\nThe separate -sources archive is optional for running the player.\n", encoding="utf-8")
    (sources / "README.txt").write_text("Source code of bundled MPL dependencies and, on Linux, the matching ALSA source package. Build tools: Rust/Cargo and a C compiler. Application source and Cargo.lock are available at the release tag in the repository.\n", encoding="utf-8")
    dist = root / "dist"
    dist.mkdir(exist_ok=True)
    if is_windows:
        archive(bundle, dist / (name + ".zip"))
        archive(sources, dist / (name + "-sources.zip"))
    else:
        archive(bundle, dist / (name + ".tar.gz"))
        archive(sources, dist / (name + "-sources.tar.gz"))


if __name__ == "__main__":
    main()
