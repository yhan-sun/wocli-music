"""Paths and process entry points for source installs and portable bundles."""

import os
import sys
from pathlib import Path


def frozen() -> bool:
    return bool(getattr(sys, "frozen", False))


def bundled_mpv() -> Path | None:
    if frozen():
        return Path(sys._MEIPASS) / "mpv" / "mpv"
    return None


def service_command() -> list[str]:
    if frozen():
        return [sys.executable, "--wocli-service"]
    return [sys.executable, "-m", "wocli_music.service"]


def configure_certificates():
    if frozen():
        import certifi

        os.environ.setdefault("SSL_CERT_FILE", certifi.where())
        os.environ.setdefault("CURL_CA_BUNDLE", certifi.where())
