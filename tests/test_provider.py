import hashlib
import io
import threading
import zlib

import pytest

from wocli_music.models import AudioUnavailable, IncompleteAudio, MusicError, Stream, Track
from wocli_music.provider import KuwoProvider, decode_reply, encode_query


class Response(io.BytesIO):
    def __init__(self, data, headers=None, status=206):
        super().__init__(data)
        self.headers = headers or {}
        self.status = status


def test_protocol_matches_legacy_pipeline():
    # Fixed vector produced by the existing ../music/kw_full.py implementation.
    plaintext = "<001><U_QRY>|<11,22>|<test>"
    encoded = b"LkxQZR54BSdoPko3RgQvQjdtfz9UBT4lGkUh"
    assert encode_query(plaintext) == encoded
    assert decode_reply(encoded) == plaintext.encode()
    with pytest.raises(MusicError):
        decode_reply(b"")


def test_details_accept_legacy_python_response_without_search(monkeypatch):
    provider = KuwoProvider()
    raw = b"{'abslist':[{'MUSICRID':'MUSIC_228908','SONGNAME':'Song','DURATION':'269'}]}"
    monkeypatch.setattr(provider, "_read", lambda *a, **k: raw)
    track = provider.details("228908")
    assert track.id == "kuwo:228908"
    assert track.title == "Song"
    assert track.duration == 269


def test_search_accepts_legacy_python_dict_and_normalizes_text(monkeypatch):
    provider = KuwoProvider()
    body = "{'TOTAL': '1', 'abslist': [{'MUSICRID':'MUSIC_1', 'SONGNAME':'<b>晴天</b>&nbsp;', 'ARTIST':'周杰伦', 'DURATION':'269'}]}"
    monkeypatch.setattr(provider, "_read", lambda *a, **k: body.encode())
    total, tracks = provider.search("晴天")
    assert total == 1
    assert tracks[0].id == "kuwo:1"
    assert tracks[0].title == "晴天"


def test_network_error_is_not_an_empty_search(monkeypatch):
    provider = KuwoProvider()
    monkeypatch.setattr(provider, "_read", lambda *a, **k: b"<html>blocked</html>")
    with pytest.raises(MusicError, match="无法读取"):
        provider.search("song")


def test_quality_metadata_is_parsed_from_binary_envelope(monkeypatch, tracks):
    provider = KuwoProvider()
    payload = zlib.compress(b"MUSICRID=MUSIC_1\nMP3H=S1:11|S2:22|SIZE:98765|BT:320\n")
    monkeypatch.setattr(
        provider, "_read", lambda *a, **k: b"head" + len(payload).to_bytes(4, "little") + payload
    )
    assert provider.qualities(tracks[0]) == [
        {"quality": "320k", "s1": 11, "s2": 22, "size": 98765, "extension": ".mp3"}
    ]


@pytest.mark.parametrize(
    "body,headers",
    [
        (b"<html>forbidden</html>", {"Content-Type": "audio/mpeg"}),
        (b"ID3" + b"x" * 100, {"Content-Range": "bytes 0-102/103"}),
    ],
)
def test_probe_rejects_html_and_short_preview(monkeypatch, body, headers):
    provider = KuwoProvider()
    monkeypatch.setattr(provider, "_open", lambda *a, **k: Response(body, headers))
    with pytest.raises(MusicError):
        provider._probe("https://cdn.example/song", 10000)


def test_quality_fallback_preserves_song_identity(monkeypatch, tracks):
    provider = KuwoProvider()
    monkeypatch.setattr(
        provider,
        "qualities",
        lambda track: [
            {"quality": "flac", "extension": ".flac", "size": 10000},
            {"quality": "320k", "extension": ".mp3", "size": 1000},
            {"quality": "128k", "extension": ".mp3", "size": 500},
        ],
    )
    attempted = []

    def resolve(quality):
        attempted.append(quality["quality"])
        if quality["quality"] != "128k":
            raise MusicError("unavailable")
        return "https://cdn.example/1.mp3", 500

    monkeypatch.setattr(provider, "_resolve", resolve)
    monkeypatch.setattr(provider, "_probe", lambda *a: None)
    result = provider.stream(tracks[0], "320k")
    assert result.quality == "128k"
    assert attempted == ["320k", "128k"]


def test_download_checks_md5_before_replacing_existing_file(monkeypatch, tmp_path):
    provider = KuwoProvider()
    content = b"ID3" + b"x" * 100
    headers = {
        "Content-Range": f"bytes 0-{len(content) - 1}/{len(content)}",
        "X-COS-META-MD5": "0" * 32,
    }
    monkeypatch.setattr(provider, "_open", lambda *a, **k: Response(content, headers))
    target = tmp_path / "song.mp3"
    target.write_bytes(b"old content")
    with pytest.raises(MusicError, match="MD5"):
        provider.download(
            Stream("https://cdn.example/1.mp3", "128k", len(content), ".mp3"),
            target,
            overwrite=True,
        )
    assert target.read_bytes() == b"old content"
    assert not list(tmp_path.glob("*.part"))
    headers["X-COS-META-MD5"] = hashlib.md5(content).hexdigest()
    provider.download(
        Stream("https://cdn.example/1.mp3", "128k", len(content), ".mp3"), target, overwrite=True
    )
    assert target.read_bytes() == content


def test_download_refuses_wrong_range_start(monkeypatch, tmp_path):
    provider = KuwoProvider()
    monkeypatch.setattr(
        provider, "_open", lambda *a, **k: Response(b"ID3", {"Content-Range": "bytes 100-102/103"})
    )
    with pytest.raises(MusicError, match="分块"):
        provider.download(
            Stream("https://cdn.example/song", "128k", 103, ".mp3"), tmp_path / "song.mp3"
        )


def test_local_stream_does_not_use_network(tmp_path):
    target = tmp_path / "中文 音乐.flac"
    target.write_bytes(b"fLaC")
    track = Track.local(str(target))
    assert KuwoProvider().stream(track).url == str(target)


def test_cancelled_download_removes_partial_file(monkeypatch, tmp_path):
    provider = KuwoProvider()
    cancelled = threading.Event()
    content = b"ID3" + b"x" * 100
    monkeypatch.setattr(
        provider,
        "_open",
        lambda *a, **k: Response(
            content, {"Content-Range": f"bytes 0-{len(content) - 1}/{len(content)}"}
        ),
    )
    target = tmp_path / "song.mp3"
    with pytest.raises(MusicError, match="取消"):
        provider.download(
            Stream("https://cdn.example/1.mp3", "128k", len(content) * 2, ".mp3"),
            target,
            cancelled=cancelled,
            progress=lambda *_: cancelled.set(),
        )
    assert not target.exists()
    assert not list(tmp_path.glob("*.part"))


def test_public_endpoint_fallback_uses_same_track_and_validates_full_audio(monkeypatch, tracks):
    provider = KuwoProvider()
    quality = {"quality": "128k", "extension": ".mp3", "size": 3587615}
    monkeypatch.setattr(provider, "qualities", lambda _: [quality])

    def no_p2p(_):
        raise AudioUnavailable("该档位未提供播放地址")

    monkeypatch.setattr(provider, "_resolve", no_p2p)
    requests = []

    def metadata(url):
        requests.append(url)
        return {"code": 200, "url": "https://cdn.example/full.mp3"}

    monkeypatch.setattr(provider, "_json", metadata)
    monkeypatch.setattr(
        provider,
        "_open",
        lambda *a, **k: Response(b"ID3" + b"x" * 4093, {"Content-Range": "bytes 0-4095/3587615"}),
    )
    stream = provider.stream(tracks[0], "128k")
    assert stream.quality == "128k"
    assert stream.size == 3587615
    assert "rid=1" in requests[0]
    assert "br=128k" in requests[0]


def test_cry_for_me_preview_is_not_accepted_as_a_complete_track(monkeypatch):
    provider = KuwoProvider()
    track = Track("kuwo:454255595", "Cry For Me (Explicit)", "The Weeknd", duration=224)
    qualities = [
        {"quality": "320k", "extension": ".mp3", "size": 8968130},
        {"quality": "128k", "extension": ".mp3", "size": 3587615},
    ]
    monkeypatch.setattr(provider, "qualities", lambda _: qualities)

    def no_p2p(_):
        raise AudioUnavailable("该档位未提供播放地址")

    monkeypatch.setattr(provider, "_resolve", no_p2p)
    monkeypatch.setattr(provider, "_public_url", lambda *_: "https://cdn.example/preview.mp3")
    # Captured from the public endpoint: ~181 KB instead of the 3.6 MB full 128k track.
    monkeypatch.setattr(
        provider,
        "_open",
        lambda *a, **k: Response(
            bytes.fromhex("fffb90c4") + b"x" * 4092, {"Content-Range": "bytes 0-4095/181521"}
        ),
    )
    with pytest.raises(IncompleteAudio, match="完整版") as error:
        provider.stream(track, "320k")
    assert error.value.code == "incomplete_audio"
    assert len(error.value.details) == 4
    assert "公开接口 128k" in error.value.details[-1]


def test_missing_public_url_is_reported_as_unavailable(monkeypatch, tracks):
    provider = KuwoProvider()
    monkeypatch.setattr(provider, "_json", lambda *_: {"code": 200, "url": ""})
    with pytest.raises(AudioUnavailable):
        provider._public_url(tracks[0], {"quality": "128k"})
