import sys

from wocli_music import runtime
from wocli_music.config import mpv_command


def test_background_entry_uses_python_for_source_and_service_flag_when_frozen(monkeypatch):
    monkeypatch.delattr(sys, "frozen", raising=False)
    assert runtime.service_command() == [sys.executable, "-m", "wocli_music.service"]
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    assert runtime.service_command() == [sys.executable, "--wocli-service"]


def test_portable_mpv_precedes_path_and_explicit_override_still_wins(tmp_path, monkeypatch):
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setattr(sys, "_MEIPASS", str(tmp_path), raising=False)
    monkeypatch.delenv("WOCLI_MPV", raising=False)
    monkeypatch.setenv("PATH", "/no-system-tools")
    executable = tmp_path / "mpv/mpv"
    executable.parent.mkdir()
    executable.write_text("test")
    executable.chmod(0o755)
    assert mpv_command() == str(executable)
    monkeypatch.setenv("WOCLI_MPV", "/custom/mpv")
    assert mpv_command() == "/custom/mpv"
