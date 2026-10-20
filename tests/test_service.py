import asyncio
import time

import pytest
from conftest import FakePlayer, FakeProvider, loaded

from wocli_music.models import IncompleteAudio, MusicError, Track
from wocli_music.service import MusicService


async def test_queue_controls_and_persistence(service, tracks):
    await service.dispatch({"op": "play", "tracks": [tracks[0].data()], "index": 0})
    await loaded(service)
    assert service.status()["phase"] == "playing"
    await service.dispatch({"op": "queue_add", "tracks": [tracks[1].data()]})
    await service.dispatch({"op": "pause"})
    assert service.status()["phase"] == "paused"
    await service.dispatch({"op": "next"})
    await loaded(service)
    assert service.current.id == tracks[1].id
    assert service.player.loads == [tracks[0].id, tracks[1].id]
    saved = service.store.load("queue")
    assert saved["index"] == 1
    assert len(saved["tracks"]) == 2


async def test_stop_during_resolution_cannot_start_late_audio(paths, tracks):
    class SlowProvider(FakeProvider):
        def stream(self, track, quality):
            time.sleep(0.08)
            return super().stream(track, quality)

    service = MusicService(paths, provider=SlowProvider(), player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await asyncio.sleep(0.01)
        await service.dispatch({"op": "stop"})
        await asyncio.sleep(0.12)
        assert service.phase == "stopped"
        assert service.player.loads == []
    finally:
        await service.close()


async def test_new_selection_wins_over_old_slow_resolution(paths, tracks):
    class SlowProvider(FakeProvider):
        def stream(self, track, quality):
            if track.id == tracks[0].id:
                time.sleep(0.1)
            return super().stream(track, quality)

    service = MusicService(paths, provider=SlowProvider(), player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await asyncio.sleep(0.01)
        await service.dispatch({"op": "play", "tracks": [tracks[1].data()], "index": 0})
        await loaded(service)
        await asyncio.sleep(0.12)
        assert service.current.id == tracks[1].id
        assert service.player.loads == [tracks[1].id]
    finally:
        await service.close()


async def test_eof_advances_once_and_ignores_replaced_file(service, tracks):
    await service.dispatch({"op": "play", "tracks": [t.data() for t in tracks], "index": 0})
    await loaded(service)
    service.entry_id = 42
    await service._handle_end(
        {"event": "end-file", "reason": "eof", "playlist_entry_id": 41}, service.generation
    )
    assert service.index == 0
    event = {"event": "end-file", "reason": "eof", "playlist_entry_id": 42}
    generation = service.generation
    await asyncio.gather(
        service._handle_end(event, generation), service._handle_end(event, generation)
    )
    await loaded(service)
    assert service.index == 1
    assert service.player.loads == [tracks[0].id, tracks[1].id]
    await service._handle_end(event, service.generation)
    assert service.phase == "stopped"


@pytest.mark.parametrize("mode,expected", [("repeat", 0), ("single", 1), ("shuffle", 0)])
async def test_loop_modes(service, tracks, mode, expected):
    await service.dispatch({"op": "play", "tracks": [t.data() for t in tracks], "index": 1})
    await loaded(service)
    await service.dispatch({"op": "mode", "value": mode})
    await service._handle_end({"event": "end-file", "reason": "eof"}, service.generation)
    await loaded(service)
    assert service.index == expected


async def test_unavailable_track_reports_error_without_substitution(service):
    track = Track("kuwo:404", "不可用歌曲")
    await service.dispatch({"op": "play", "tracks": [track.data()]})
    await loaded(service)
    assert service.status()["phase"] == "error"
    assert service.status()["track"]["id"] == track.id
    assert service.player.loads == []


async def test_invalid_volume_and_queue_index_do_not_change_state(service, tracks):
    with pytest.raises(MusicError):
        await service.dispatch({"op": "volume", "value": 999})
    assert service.status()["volume"] == 60
    await service.dispatch({"op": "queue_add", "tracks": [tracks[0].data()]})
    with pytest.raises(MusicError):
        await service.dispatch({"op": "queue_remove", "index": -1})
    assert len(service.queue) == 1


async def test_invalid_replacement_keeps_current_queue(service, tracks):
    await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
    await loaded(service)
    with pytest.raises(MusicError):
        await service.dispatch({"op": "play", "tracks": [tracks[1].data()], "index": 999})
    assert service.current.id == tracks[0].id
    assert service.phase == "playing"


async def test_short_preview_is_stopped_before_marking_playing(paths, tracks):
    class PreviewPlayer(FakePlayer):
        async def load(self, url, volume):
            await super().load(url, volume)
            self.properties["duration"] = 11

    service = MusicService(paths, provider=FakeProvider(), player_factory=PreviewPlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        assert service.phase == "error"
        assert "试听" in service.error
        assert service.status()["error_code"] == "incomplete_audio"
        assert service.player.commands[-1] == ("stop",)
        assert service.store.history() == []
    finally:
        await service.close()


async def test_unavailable_source_details_are_exposed_without_starting_audio(paths, tracks):
    class PreviewProvider(FakeProvider):
        def stream(self, track, quality):
            raise IncompleteAudio(
                "仅返回试听，未获取到完整版。", ["P2P 320k: 无地址", "公开接口 320k: 试听"]
            )

    service = MusicService(paths, provider=PreviewProvider(), player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        state = service.status()
        assert state["phase"] == "error"
        assert state["error_code"] == "incomplete_audio"
        assert len(state["error_details"]) == 2
        await service.lyric_task
        assert service.status()["lyrics_status"] == "ready"
        assert service.player.loads == []
    finally:
        await service.close()


async def test_lyric_exception_leaves_loading_without_stopping_playback(paths, tracks):
    class BrokenLyrics(FakeProvider):
        def lyrics(self, track):
            raise AttributeError("malformed upstream schema")

    service = MusicService(paths, provider=BrokenLyrics(), player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        await service.lyric_task
        assert service.phase == "playing"
        assert service.lyrics_status == "unavailable"
        assert "失败" in service.lyrics_error
        assert service.lyric_text == ""
    finally:
        await service.close()


async def test_lyric_deadline_leaves_loading_and_discards_late_result(paths, tracks, monkeypatch):
    class SlowLyrics(FakeProvider):
        def lyrics(self, track):
            time.sleep(0.06)
            return "[00:01]late"

    monkeypatch.setattr("wocli_music.service.LYRICS_TIMEOUT", 0.01)
    service = MusicService(paths, provider=SlowLyrics(), player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        await service.lyric_task
        assert service.phase == "playing"
        assert service.lyrics_status == "unavailable"
        assert "超时" in service.lyrics_error
        await asyncio.sleep(0.08)
        assert service.lyric_text == ""
        assert service.store.lyric(tracks[0].id) == ""
    finally:
        await service.close()


async def test_shuffle_navigation_rounds_and_queue_edits(service):
    tracks = [Track(f"kuwo:{i}", f"歌曲 {i}", duration=269) for i in range(10, 15)]
    await service.dispatch({"op": "play", "tracks": [track.data() for track in tracks]})
    await loaded(service)
    await service.dispatch({"op": "mode", "value": "shuffle"})
    played = [service.current.id]
    for _ in range(9):
        await service.dispatch({"op": "next"})
        await loaded(service)
        played.append(service.current.id)
    assert set(played[:5]) == set(played[5:]) == {track.id for track in tracks}
    await service.dispatch({"op": "prev"})
    await loaded(service)
    assert service.current.id == played[-2]
    await service.dispatch({"op": "next"})
    await loaded(service)
    assert service.current.id == played[-1]
    removed = next(i for i in range(5) if i != service.index)
    await service.dispatch({"op": "queue_remove", "index": removed})
    remaining = {track.id for track in service.queue}
    for _ in range(4):
        await service.dispatch({"op": "next"})
        await loaded(service)
        assert service.current.id in remaining


async def test_transient_audio_failure_retries_once_without_replacing_queue(
    paths, tracks, monkeypatch
):
    from wocli_music.models import NetworkError

    class Flaky(FakeProvider):
        def __init__(self):
            self.calls = 0

        def stream(self, track, quality):
            self.calls += 1
            if self.calls == 1:
                raise NetworkError("网络暂时中断")
            return super().stream(track, quality)

    monkeypatch.setattr("wocli_music.service.RETRY_DELAY", 0)
    provider = Flaky()
    service = MusicService(paths, provider=provider, player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [track.data() for track in tracks]})
        await loaded(service)
        assert provider.calls == 2
        assert service.phase == "playing"
        assert service.queue == tracks
        assert service.index == 0
        assert service.status()["retry_attempt"] == 2
    finally:
        await service.close()


async def test_audio_retry_keeps_current_track_when_selection_changes_and_guards_busy(
    service, tracks
):
    await service.dispatch({"op": "play", "tracks": [t.data() for t in tracks], "index": 1})
    await loaded(service)
    await service.dispatch({"op": "retry", "quality": "best"})
    task = service.play_task
    await service.dispatch({"op": "retry"})
    assert service.play_task is task
    await loaded(service)
    assert service.queue == tracks
    assert service.current == tracks[1]
    assert service.status()["requested_quality"] == "best"


async def test_lyrics_retry_is_independent_bounded_and_keeps_cached_lines(
    paths, tracks, monkeypatch
):
    class Slow(FakeProvider):
        def lyrics(self, track):
            time.sleep(0.06)
            return "[00:01]too late"

    monkeypatch.setattr("wocli_music.service.LYRICS_TIMEOUT", 0.01)
    service = MusicService(paths, provider=Slow(), player_factory=FakePlayer)
    try:
        service.store.lyric(tracks[0].id, "[00:01]cached line")
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        loads = list(service.player.loads)
        await service.dispatch({"op": "lyrics_retry"})
        task = service.lyric_task
        await service.dispatch({"op": "lyrics_retry"})
        assert service.lyric_task is task
        await task
        assert service.lyrics_status == "ready"
        assert "超时" in service.lyrics_error
        assert service.lyric_text == "[00:01]cached line"
        assert service.phase == "playing"
        assert service.player.loads == loads
        await asyncio.sleep(0.08)
        assert service.lyric_text == "[00:01]cached line"
    finally:
        await service.close()


async def test_source_deadline_exhausts_once_and_stop_discards_late_audio(
    paths, tracks, monkeypatch
):
    class Slow(FakeProvider):
        def __init__(self):
            self.calls = 0

        def stream(self, track, quality):
            self.calls += 1
            time.sleep(0.06)
            return super().stream(track, quality)

    monkeypatch.setattr("wocli_music.service.SOURCE_TIMEOUT", 0.01)
    monkeypatch.setattr("wocli_music.service.RETRY_DELAY", 0)
    provider = Slow()
    service = MusicService(paths, provider=provider, player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        assert provider.calls == 2
        assert service.phase == "error"
        assert "超时" in service.error
        await asyncio.sleep(0.08)
        assert service.player.loads == []
        await service.dispatch({"op": "retry"})
        await service.dispatch({"op": "stop"})
        await asyncio.sleep(0.08)
        assert service.phase == "stopped"
        assert service.player.loads == []
    finally:
        await service.close()


async def test_retrying_audio_does_not_restart_inflight_lyrics(paths, tracks):
    import threading

    release = threading.Event()

    class SlowLyrics(FakeProvider):
        def lyrics(self, track):
            release.wait(2)
            return "[00:01]one independent lyric request"

    service = MusicService(paths, provider=SlowLyrics(), player_factory=FakePlayer)
    try:
        await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
        await loaded(service)
        task = service.lyric_task
        await service.dispatch({"op": "retry", "quality": "best"})
        await loaded(service)
        assert service.lyric_task is task
        release.set()
        await task
        assert service.lyrics_status == "ready"
        assert service.lyric_text.endswith("one independent lyric request")
    finally:
        release.set()
        await service.close()
