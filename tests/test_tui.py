import pytest
from textual.widgets import DataTable, Input, Select, Static

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
        app.action_search()
        await pilot.pause()
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
        assert len(service.queue) == 3
        await pilot.press("ctrl+f")
        assert isinstance(app.focused, Input)
        search.value = ""
        await pilot.press("q", "space", "a", "f")
        assert search.value == "q af"
        if size[0] < 110:
            assert not app.query_one("#now").display
        assert app.query_one("#play-pause").region.x >= 0
        assert app.query_one("#favorite").region.right <= size[0]


async def test_tui_handles_empty_and_failed_search(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.provider = UIProvider(tracks)
    app.client = UIClient(service)
    async with app.run_test() as pilot:
        app.action_search()
        await pilot.pause()
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


async def test_library_search_is_collapsed_and_esc_returns_to_folder(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.store.add_to_playlist("通勤", tracks[0])
    async with app.run_test(size=(123, 41)) as pilot:
        assert not app.query_one("#searchbar").display
        app.show_view("playlist:通勤")
        await pilot.press("slash")
        await pilot.pause()
        assert app.query_one("#searchbar").display
        assert isinstance(app.focused, Input)
        await pilot.press("escape")
        await pilot.pause()
        assert app.active_view == "playlist:通勤"
        assert not app.query_one("#searchbar").display
        assert app.rows["results"] == [tracks[0]]


async def test_folder_create_collect_rename_and_delete(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.store.favorite(tracks[0])
    async with app.run_test(size=(123, 41)) as pilot:
        app.action_new_folder()
        await pilot.pause()
        app.screen.query_one("#folder-name", Input).value = "通勤"
        await pilot.press("enter")
        await pilot.pause()
        assert app.active_view == "playlist:通勤"
        app.show_view("favorites")
        app.action_collect()
        await pilot.pause()
        assert app.screen.query_one("#folder-select", Select).value == "folder:通勤"
        await pilot.click("#save")
        await pilot.pause()
        assert app.store.playlist("通勤") == [tracks[0]]
        app.show_view("playlist:通勤")
        app.action_rename_folder()
        await pilot.pause()
        app.screen.query_one("#folder-name", Input).value = "路上"
        await pilot.press("enter")
        await pilot.pause()
        assert app.active_view == "playlist:路上"
        assert app.store.playlist("路上") == [tracks[0]]
        app.delete_folder()
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
        assert app.store.playlist("路上") == [tracks[0]]
        app.delete_folder()
        await pilot.pause()
        await pilot.click("#delete-confirm")
        await pilot.pause()
        assert app.store.playlists() == []
        assert app.store.favorites() == [tracks[0]]
        assert app.active_view == "favorites"


@pytest.mark.parametrize(
    "key,mode", [("1", "list"), ("2", "repeat"), ("3", "single"), ("4", "shuffle")]
)
async def test_mode_picker_selects_and_persists(paths, service, key, mode):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    async with app.run_test(size=(100, 32)) as pilot:
        app.action_mode()
        await pilot.pause()
        await pilot.press(key)
        await pilot.pause()
        assert service.settings.values["mode"] == mode
        from wocli_music.config import Settings

        assert Settings(paths).values["mode"] == mode


async def test_folder_plays_whole_collection_and_remove_preserves_other_folder(
    paths, service, tracks
):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    for track in tracks:
        app.store.add_to_playlist("工作", track)
        app.store.add_to_playlist("备份", track)
    async with app.run_test(size=(120, 38)) as pilot:
        app.show_view("playlist:工作")
        app.query_one("#results", DataTable).focus()
        await pilot.press("enter")
        await pilot.pause()
        assert service.queue == tracks
        app.action_remove_track()
        assert app.store.playlist("工作") == [tracks[1]]
        assert app.store.playlist("备份") == tracks


async def test_small_terminal_library_menu_and_modal_shortcuts(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.store.add_to_playlist("旅途", tracks[0])
    async with app.run_test(size=(60, 24)) as pilot:
        assert app.query_one("#library-open").display
        await pilot.click("#library-open")
        await pilot.pause()
        await pilot.press("down", "down", "down", "enter")
        await pilot.pause()
        assert app.active_view == "playlist:旅途"
        assert app.selected() == tracks[0]
        app.action_mode()
        await pilot.pause()
        # Main-screen shortcuts must not run behind the modal.
        await pilot.press("f", "c", "n", "delete")
        await pilot.pause()
        assert not app.store.is_favorite(tracks[0].id)
        assert app.store.playlist("旅途") == [tracks[0]]
        assert app.screen.__class__.__name__ == "ModePicker"
        dialog = app.screen.query_one(".dialog")
        assert dialog.region.x > 0
        assert dialog.region.bottom <= 24
        await pilot.press("escape")
        await pilot.pause()


async def test_collect_can_create_first_folder_and_preserves_pinned_track(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.search_tracks = tracks
    async with app.run_test(size=(100, 32)) as pilot:
        app.show_view("search")
        app.query_one("#results", DataTable).move_cursor(row=1)
        app.action_collect()
        await pilot.pause()
        # Polling continues while the picker is open; the chosen track stays pinned.
        app.poll()
        await pilot.pause()
        app.screen.query_one("#folder-new", Input).value = "旅行"
        await pilot.press("enter")
        await pilot.pause()
        assert app.store.playlist("旅行") == [tracks[1]]
        assert app.store.load("ui.last_folder") == "旅行"


async def test_periodic_refresh_keeps_sidebar_navigation_and_duplicate_queue_cursor(
    paths, service, tracks
):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    service.queue = [tracks[0], tracks[1], tracks[0]]
    async with app.run_test(size=(120, 38)) as pilot:
        await pilot.pause()
        app.show_view("queue")
        table = app.query_one("#results", DataTable)
        table.move_cursor(row=2)
        app.refresh_library()
        assert table.cursor_row == 2
        nav = app.query_one("#library")
        nav.focus()
        nav.highlighted = 2
        app.refresh_library()
        assert nav.highlighted == 2


async def test_search_completion_while_modal_is_open_is_retained(paths, service, tracks):
    import time

    class SlowSearch(UIProvider):
        def search(self, *args):
            time.sleep(0.1)
            return super().search(*args)

    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.provider = SlowSearch(tracks)
    async with app.run_test(size=(120, 38)) as pilot:
        app.action_search()
        app.query_one("#search", Input).value = "晴天"
        app.start_search()
        await pilot.pause(0.02)
        app.action_mode()
        await pilot.pause(0.2)
        await app.workers.wait_for_complete()
        assert app.screen.__class__.__name__ == "ModePicker"
        await pilot.press("escape")
        await pilot.pause()
        app.refresh_library()
        assert app.rows["results"] == tracks
        assert "2 首" in str(app.query_one("#list-status", Static).render())
