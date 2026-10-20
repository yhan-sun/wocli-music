import urllib.parse

import pytest

from wocli_music.lyrics import lines_to_lrc, lyric_matches, lyric_title, parse_lrc
from wocli_music.models import Track
from wocli_music.provider import KuwoProvider


@pytest.fixture
def majiko():
    return Track("kuwo:61631119", "心做し (カバー)", "majiko", "Contrast", 273)


def test_native_lyrics_are_requested_with_mobile_headers_and_exact_identity(monkeypatch, majiko):
    provider = KuwoProvider()
    calls = []

    def get(url, **kwargs):
        calls.append(url)
        assert "musicId=61631119" in url
        assert kwargs["headers"]["Referer"] == "https://m.kuwo.cn/h5app/"
        assert "Mobile" in kwargs["headers"]["User-Agent"]
        return {
            "status": 200,
            "data": {
                "lrclist": [
                    {"time": "1.25", "lineLyric": "テスト歌詞"},
                    {"time": "63.50", "lineLyric": "第二行"},
                ]
            },
        }

    monkeypatch.setattr(provider, "_json", get)
    text = provider.lyrics(majiko)
    assert [line.time for line in parse_lrc(text)] == [1.25, 63.5]
    assert len(calls) == 1


def test_cover_annotation_falls_back_to_matching_studio_lyrics(monkeypatch, majiko):
    provider = KuwoProvider()
    calls = []

    def get(url, **kwargs):
        calls.append(url)
        if "kuwo.cn/newh5" in url:
            return {"status": 301, "data": None}
        if "krcs.kugou.com" in url:
            return {"candidates": [{"song": "心做し", "singer": "majiko", "duration": 248816}]}
        if "/search/get/web" in url:
            keyword = urllib.parse.parse_qs(urllib.parse.urlsplit(url).query)["s"][0]
            assert "カバー" not in keyword
            return {
                "result": {
                    "songs": [
                        {
                            "id": 1,
                            "name": "心做し (Live At 渋谷WWW X / 2017)",
                            "artists": [{"name": "majiko"}],
                            "duration": 305542,
                        },
                        {
                            "id": 2,
                            "name": "心做し",
                            "artists": [{"name": "其他歌手"}],
                            "duration": 273583,
                        },
                        {
                            "id": 3,
                            "name": "心做し",
                            "artists": [{"name": "majiko"}],
                            "duration": 273583,
                        },
                        {
                            "id": 4,
                            "name": "心做し",
                            "artists": [{"name": "majiko"}],
                            "duration": 273583,
                        },
                    ]
                }
            }
        if "id=3" in url:
            return {"lrc": None}
        if "id=4" in url:
            return {"lrc": {"lyric": "[00:01.00]テスト歌詞\n[00:02.50]次の行"}}
        pytest.fail(f"Unexpected lyric request: {url}")

    monkeypatch.setattr(provider, "_json", get)
    text = provider.lyrics(majiko)
    assert len(parse_lrc(text)) == 2
    assert not any("&id=1" in url or "&id=2" in url for url in calls)


def test_malformed_native_data_does_not_stop_other_lyric_sources(monkeypatch, majiko):
    provider = KuwoProvider()

    def get(url, **kwargs):
        if "kuwo.cn" in url:
            return {"data": {"lrclist": [None, {"time": "NaN", "lineLyric": "invalid"}]}}
        if "krcs.kugou.com" in url:
            return {"candidates": None}
        return {"result": None}

    monkeypatch.setattr(provider, "_json", get)
    assert provider.lyrics(majiko) == ""


def test_annotation_normalization_keeps_live_and_remix_versions(majiko):
    assert lyric_title("心做し（カバー）") == "心做し"
    assert lyric_title("Song (Explicit)") == "Song"
    assert "Live" in lyric_title("心做し (2017 Live Version)")
    assert "Remix" in lyric_title("Song (Remix)")
    assert lyric_matches(majiko, "心做し", ["majiko"], 273.583)
    assert not lyric_matches(majiko, "心做し (Live)", ["majiko"], 273.583)
    assert not lyric_matches(majiko, "心做し", ["majiko"], 248.816)
    assert not lyric_matches(majiko, "心做し", ["someone else"], 273.583)


def test_native_timestamp_conversion_skips_invalid_entries():
    text = lines_to_lrc(
        [
            None,
            {},
            {"time": float("inf"), "lineLyric": "invalid"},
            {"time": -1, "lineLyric": "invalid"},
            {"time": "61.234", "lineLyric": "valid"},
        ]
    )
    assert text == "[01:01.23]valid"


def test_native_legacy_transport_is_used_when_https_returns_no_music(monkeypatch, majiko):
    provider = KuwoProvider()
    calls = []

    def get(url, **kwargs):
        calls.append(url)
        if url.startswith("https:"):
            return {"status": 301, "msg": "音乐查询失败", "data": None}
        return {"status": 200, "data": {"lrclist": [{"time": "1.2", "lineLyric": "歌词"}]}}

    monkeypatch.setattr(provider, "_json", get)
    assert parse_lrc(provider.lyrics(majiko))[0].time == 1.2
    assert len(calls) == 2
    assert calls[1].startswith("http://m.kuwo.cn/")
