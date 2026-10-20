import pytest
from textual.widgets import DataTable, Input, Static, TabbedContent

from wocli_music.models import MusicError
from wocli_music.tui import MusicApp


class UIProvider:
    def __init__(self, tracks):
        self.tracks = tracks

    def search(self, keyword, page=0, limit=20):
        if keyword == "error":
            raise MusicError("测试网络错误")
        if keyword == "empty":
            return 0, []
        return len(self.tracks), self.tracks


class UIClient:
    def __init__(self, service):
        self.service = service

    async def request(self, operation, autostart=True, **kwargs):
        return await self.service.dispatch({"op": operation, **kwargs})


@pytest.mark.parametrize("size", [(120, 38), (80, 26), (60, 24)])
async def test_tui_search_play_and_responsive_layout(paths, service, tracks, size):
    app = MusicApp(paths=paths)
    app.provider = UIProvider(tracks)
    app.client = UIClient(service)
    async with app.run_test(size=size) as pilot:
        search = app.query_one("#search", Input)
        search.value = "周杰伦"
        await pilot.press("enter")
        await pilot.pause()
        assert app.query_one("#results", DataTable).row_count == 2
        assert isinstance(app.focused, DataTable)
        await pilot.press("enter")
        await pilot.pause()
        if service.play_task:
            await service.play_task
        app.poll()
        await pilot.pause()
        assert service.current.id == tracks[0].id
        assert service.phase == "playing"
        await pilot.press("space")
        await pilot.pause()
        assert service.phase == "paused"
        await pilot.press("f")
        await pilot.pause()
        assert service.store.is_favorite(tracks[0].id)
        await pilot.press("a")
        await pilot.pause()
        assert len(service.queue) == 2
        await pilot.press("ctrl+f")
        assert isinstance(app.focused, Input)
        search.value = ""
        await pilot.press("q", "space", "a", "f")
        assert search.value == "q af"
        if size[0] < 96:
            assert not app.query_one("#now").display
        assert app.query_one("#play-pause").region.x >= 0
        assert app.query_one("#favorite").region.right <= size[0]


async def test_tui_handles_empty_and_failed_search(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.provider = UIProvider(tracks)
    app.client = UIClient(service)
    async with app.run_test() as pilot:
        search = app.query_one("#search", Input)
        search.value = "empty"
        await pilot.press("enter")
        await pilot.pause()
        assert app.query_one("#results", DataTable).row_count == 0
        search.focus()
        search.value = "error"
        await pilot.press("enter")
        await pilot.pause()
        assert "测试网络错误" in str(app.query_one("#list-status", Static).render())
        app.query_one("#tabs", TabbedContent).active = "tab-playlists"
        await pilot.pause()
        app.query_one("#playlist-name", Input).value = "工作"
        await pilot.click("#playlist-create")
        await pilot.pause()
        assert app.store.playlists() == [{"name": "工作", "count": 0}]


async def test_tui_replaces_lyric_loading_when_playback_fails(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    async with app.run_test(size=(120, 38)) as pilot:
        await pilot.pause()
        service.queue = [tracks[0]]
        service.index = 0
        service.phase = "loading"
        app.render_state(service.status())
        assert "正在解析音源" in str(app.query_one("#lyrics", Static).render())
        service.phase = "error"
        service.error = "该版本仅返回试听或不完整音频，未获取到可播放的完整版。"
        service.error_code = "incomplete_audio"
        app.render_state(service.status())
        content = str(app.query_one("#lyrics", Static).render())
        assert "试听" in content
        assert "正在加载" not in content
        assert "音乐仍在继续" not in content
        assert str(app.query_one("#play-pause").label) == "重试"


async def test_lyric_loading_and_unavailable_are_distinct_states(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    async with app.run_test() as pilot:
        await pilot.pause()
        service.queue = [tracks[0]]
        service.index = 0
        service.phase = "playing"
        service.lyrics_status = "loading"
        app.render_state(service.status())
        assert "正在加载歌词" in str(app.query_one("#lyrics", Static).render())
        service.lyrics_status = "unavailable"
        app.render_state(service.status())
        assert "暂未找到" in str(app.query_one("#lyrics", Static).render())


async def test_refresh_lyrics_while_playing_updates_display_without_changing_playback(
    paths, service, tracks
):
    class LyricsProvider:
        def lyrics(self, track):
            return "[00:01]新的歌词\n[00:05]下一行"

    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.provider = LyricsProvider()
    service.queue = [tracks[0]]
    service.index = 0
    service.phase = "playing"
    service.lyric_text = "[00:01]旧缓存"
    service.lyrics_status = "ready"
    async with app.run_test(size=(120, 38)) as pilot:
        await pilot.pause()
        app.state = service.status()
        app.render_state(app.state)
        app.query_one("#results", DataTable).focus()
        await pilot.press("l")
        await pilot.pause()
        await app.workers.wait_for_complete()
        assert service.phase == "playing"
        assert service.player.loads == []
        assert "新的歌词" in str(app.query_one("#lyrics", Static).render())
        assert app.store.lyric(tracks[0].id).startswith("[00:01]新的歌词")


async def test_empty_player_does_not_display_lyric_loading(paths, service):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    async with app.run_test() as pilot:
        await pilot.pause()
        app.render_state(service.status())
        assert "正在加载" not in str(app.query_one("#lyrics", Static).render())
