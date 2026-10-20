import pytest

from wocli_music.config import Settings
from wocli_music.lyrics import current_line, parse_lrc
from wocli_music.models import MusicError, clean, normalize_id
from wocli_music.store import Store


def test_lyrics_support_multiple_stamps_precision_and_offset():
    lines = parse_lrc("[ti:标题]\n[offset:100]\n[00:02.5][00:05.050]第二句\n[00:01]第一句")
    assert [line.time for line in lines] == pytest.approx([1.1, 2.6, 5.15])
    assert current_line(lines, 0) == -1
    assert current_line(lines, 2.5, 100) == 1
    assert lines[-1].text == "第二句"


def test_terminal_text_and_identity_are_normalized():
    assert clean("<b>Live</b>&nbsp;\\u0026 " + "\x1b[31mSinger") == "Live & Singer"
    assert normalize_id("MUSIC_228908") == normalize_id("kuwo:228908") == "kuwo:228908"
    with pytest.raises(MusicError):
        normalize_id("../../file")


def test_favorites_playlists_and_history_survive_restart(paths, tracks):
    store = Store(paths)
    assert store.favorite(tracks[0]) is True
    store.add_to_playlist("工作", tracks[0])
    store.add_to_playlist("工作", tracks[0])
    store.add_to_playlist("工作", tracks[1])
    store.played(tracks[0])
    store.played(tracks[1])
    store.close()
    reopened = Store(paths)
    assert [t.id for t in reopened.favorites()] == [tracks[0].id]
    assert [t.id for t in reopened.playlist("工作")] == [t.id for t in tracks]
    assert reopened.playlists() == [{"name": "工作", "count": 2}]
    assert [t.id for t in reopened.history()] == [tracks[1].id, tracks[0].id]
    assert reopened.favorite(tracks[0]) is False
    reopened.close()


def test_settings_validation_and_persistence(paths):
    settings = Settings(paths)
    with pytest.raises(MusicError):
        settings.set("volume", 101)
    with pytest.raises(MusicError):
        settings.set("quality", "imaginary")
    settings.set("quality", "128k")
    assert Settings(paths).values["quality"] == "128k"


def test_rename_delete_preserve_membership_order_and_other_collections(paths, tracks):
    store = Store(paths)
    for track in tracks:
        store.add_to_playlist("通勤", track)
        store.add_to_playlist("备份", track)
    store.favorite(tracks[0])
    store.rename_playlist("通勤", "路上")
    assert store.playlist("路上") == tracks
    assert store.playlists_for_track(tracks[0].id) == {"路上", "备份"}
    with pytest.raises(MusicError):
        store.rename_playlist("路上", "备份")
    assert store.playlist("路上") == tracks
    store.delete_playlist("路上")
    assert store.playlist("备份") == tracks
    assert store.favorites() == [tracks[0]]
    store.close()


def test_shuffle_rounds_and_previous_follow_actual_history():
    from wocli_music.order import ShuffleOrder

    order = ShuffleOrder()
    order.reset(5, 0)
    played = [0]
    for _ in range(4):
        played.append(order.next(5, played[-1]))
    assert set(played) == set(range(5))
    assert order.previous() == played[-2]
    assert order.next(5, played[-2]) == played[-1]
    for _ in range(10):
        played.append(order.next(5, played[-1]))
    assert set(played[5:10]) == set(range(5))
    assert set(played[10:15]) == set(range(5))
    assert all(left != right for left, right in zip(played, played[1:], strict=False))
    order.reset(1, 0)
    assert order.next(1, 0) == 0
