"""Build a portable directory containing wocli, Python, mpv and shared libraries.

Release builds also publish the corresponding distro/Homebrew dependency sources.
"""

import argparse
import ast
import hashlib
import importlib.metadata
import json
import platform
import re
import shutil
import subprocess
import sys
import tarfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from wocli_music import __version__
from wocli_music.config import mpv_command

ROOT = Path(__file__).resolve().parents[1]


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from strings(item)


def python_licenses(destination):
    destination.mkdir(parents=True, exist_ok=True)
    for distribution in importlib.metadata.distributions():
        for file in distribution.files or []:
            if any(
                part.lower().startswith(("license", "copying", "notice")) for part in file.parts
            ):
                source = Path(distribution.locate_file(file))
                if source.is_file():
                    target = destination / distribution.metadata["Name"] / str(file)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(source, target)
    for source in (
        Path(sys.base_prefix) / "LICENSE.txt",
        Path(sys.base_prefix)
        / "lib"
        / f"python{sys.version_info.major}.{sys.version_info.minor}"
        / "LICENSE.txt",
    ):
        if source.is_file():
            shutil.copyfile(source, destination / "Python-LICENSE.txt")


def mac_dependencies(paths, destination):
    cellar = Path(subprocess.check_output(["brew", "--cellar"], text=True).strip()).resolve()
    packages = set()
    for path in paths:
        try:
            relative = Path(path).resolve().relative_to(cellar)
        except ValueError:
            continue
        if len(relative.parts) >= 2:
            packages.add(relative.parts[0])
            package = cellar / relative.parts[0] / relative.parts[1]
            for license_file in package.rglob("*"):
                if license_file.is_file() and license_file.name.lower().startswith(
                    ("license", "copying", "notice")
                ):
                    target = destination / relative.parts[0] / license_file.relative_to(package)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(license_file, target)
            for recipe in (package / ".brew").glob("*.rb"):
                target = destination / relative.parts[0] / recipe.name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(recipe, target)
    if not packages:
        return []
    info = json.loads(
        subprocess.check_output(["brew", "info", "--json=v2", *sorted(packages)], text=True)
    )
    return [
        {
            "name": formula["name"],
            "version": formula["versions"]["stable"],
            **formula["urls"]["stable"],
            "license": formula.get("license"),
        }
        for formula in info["formulae"]
    ]


def linux_dependencies(paths, destination):
    candidates = sorted(
        {path for path in paths if path.startswith(("/usr/lib/", "/lib/", "/usr/bin/mpv"))}
    )
    result = subprocess.run(["dpkg-query", "-S", *candidates], text=True, capture_output=True)
    packages = sorted(
        {line.split(": ", 1)[0] for line in result.stdout.splitlines() if ": " in line}
    )
    if not packages:
        raise RuntimeError("Could not identify bundled mpv dependency packages")
    records = subprocess.check_output(
        [
            "dpkg-query",
            "-W",
            "-f=${binary:Package}\t${source:Package}\t${source:Version}\n",
            *packages,
        ],
        text=True,
    )
    sources = {}
    for line in records.splitlines():
        binary, source, version = line.split("\t")
        copyright_file = Path("/usr/share/doc") / binary.split(":")[0] / "copyright"
        if copyright_file.is_file():
            target = destination / binary.replace(":", "-") / "copyright"
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(copyright_file, target)
        sources[(source, version)] = {
            "name": source,
            "version": version,
            "url": f"https://launchpad.net/ubuntu/+source/{source}/{version}",
        }
    return list(sources.values())


def dependency_sources(records, destination):
    destination.mkdir(parents=True, exist_ok=True)
    if sys.platform == "linux":
        subprocess.run(
            [
                "apt-get",
                "source",
                "--download-only",
                "--only-source",
                *sorted({f"{record['name']}={record['version']}" for record in records}),
            ],
            cwd=destination,
            check=True,
        )
    else:

        def download(record):
            subprocess.run(
                ["brew", "fetch", "--build-from-source", "--retry", record["name"]], check=True
            )
            cached = Path(
                subprocess.check_output(
                    ["brew", "--cache", "--build-from-source", record["name"]], text=True
                ).strip()
            )
            if cached.is_dir():
                revision = record.get("revision") or record.get("tag")
                if not revision:
                    raise RuntimeError(f"Unpinned Git source: {record['name']}")
                target = destination / f"{record['name']}-{record['version']}.tar.gz"
                subprocess.run(
                    [
                        "git",
                        "-C",
                        str(cached),
                        "archive",
                        "--format=tar.gz",
                        "--output",
                        str(target.resolve()),
                        revision,
                    ],
                    check=True,
                )
                return
            target = destination / (record["name"] + "-" + cached.name)
            shutil.copyfile(cached, target)
            expected = record.get("checksum")
            if not expected or hashlib.sha256(target.read_bytes()).hexdigest() != expected:
                raise RuntimeError(f"Source checksum mismatch: {record['name']}")

        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(download, records))


def archive(directory, target):
    with tarfile.open(target, "w:gz") as output:
        output.add(directory, arcname=directory.name)
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    target.with_name(target.name + ".sha256").write_text(
        f"{digest}  {target.name}\n", encoding="utf8"
    )
    print(target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mpv", type=Path)
    parser.add_argument(
        "--skip-sources", action="store_true", help="Local development only; not for releases"
    )
    args = parser.parse_args()
    if sys.platform not in ("darwin", "linux"):
        raise SystemExit("Portable builds currently support macOS and Linux")
    system = "macos" if sys.platform == "darwin" else "linux"
    architecture = {"aarch64": "arm64", "amd64": "x86_64"}.get(
        platform.machine().lower(), platform.machine().lower()
    )
    name = f"wocli-music-{__version__}-{system}-{architecture}"
    work = ROOT / "build" / name
    mpv = (args.mpv or Path(mpv_command())).resolve()
    extra_binaries = []
    # Prebuilt mpv.app libraries refer to @executable_path/lib, which cannot
    # be resolved relative to each individual dylib during dependency scanning.
    for library in (mpv.parent / "lib").glob("*.dylib"):
        extra_binaries += ["--add-binary", f"{library}:."]
    if system == "linux":
        # PyInstaller normally omits the host graphics stack. mpv links some
        # of these libraries even with --no-video, so include its entire ELF
        # dependency closure except the target system's glibc and loader.
        dependencies = subprocess.check_output(["ldd", str(mpv)], text=True)
        baseline = {
            "libc.so.6",
            "libm.so.6",
            "libdl.so.2",
            "libpthread.so.0",
            "librt.so.1",
            "libresolv.so.2",
            "libutil.so.1",
            "libanl.so.1",
        }
        for soname, path in re.findall(r"(\S+)\s+=>\s+(/\S+)\s+\(", dependencies):
            if soname not in baseline:
                extra_binaries += ["--add-binary", f"{path}:."]
    subprocess.run(
        [
            sys.executable,
            "-m",
            "PyInstaller",
            "--noconfirm",
            "--clean",
            "--onedir",
            "--name",
            "wocli",
            "--distpath",
            str(work / "dist"),
            "--workpath",
            str(work / "work"),
            "--specpath",
            str(work),
            "--collect-all",
            "textual",
            "--collect-all",
            "rich",
            "--collect-all",
            "certifi",
            "--add-binary",
            f"{mpv}:mpv",
            *extra_binaries,
            str(ROOT / "scripts/standalone.py"),
        ],
        cwd=ROOT,
        check=True,
    )
    bundle = work / "dist" / "wocli"
    destination = ROOT / "dist" / name
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(bundle, destination, symlinks=True)
    for filename in ("README.md", "NOTICE.md", "CHANGELOG.md"):
        shutil.copyfile(ROOT / filename, destination / filename)
    shutil.copytree(ROOT / "docs/screenshots", destination / "docs/screenshots")
    licenses = destination / "licenses"
    python_licenses(licenses / "python")
    toc = ast.literal_eval((work / "work/wocli/Analysis-00.toc").read_text())
    paths = {value for value in strings(toc) if value.startswith("/") and Path(value).is_file()}
    paths.add(str(mpv))
    records = (mac_dependencies if system == "macos" else linux_dependencies)(
        paths, licenses / "native"
    )
    manifest = {
        "application": __version__,
        "python": platform.python_version(),
        "platform": system,
        "architecture": architecture,
        "mpv": subprocess.check_output([str(mpv), "--version"], text=True),
        "dependencies": records,
    }
    (licenses / "SOURCES.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf8"
    )
    if not args.skip_sources:
        if not records:
            raise RuntimeError("Release builds require an identified native dependency source list")
        source_dir = work / (name + "-sources")
        dependency_sources(records, source_dir)
        shutil.copyfile(licenses / "SOURCES.json", source_dir / "SOURCES.json")
        shutil.copyfile(Path(__file__), source_dir / "build_binary.py")
        shutil.copytree(licenses / "native", source_dir / "licenses", dirs_exist_ok=True)
        archive(source_dir, ROOT / "dist" / (name + "-sources.tar.gz"))
    if system == "macos":
        # Ad-hoc signing makes the generated arm64 executable runnable; this is
        # not Developer ID signing or notarization.
        subprocess.run(
            ["codesign", "--force", "--deep", "--sign", "-", str(destination / "wocli")], check=True
        )
    archive(destination, ROOT / "dist" / (name + ".tar.gz"))


if __name__ == "__main__":
    main()
