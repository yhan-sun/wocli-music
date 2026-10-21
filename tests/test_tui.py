import asyncio

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
        if size[0] < 120:
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
        assert str(app.query_one("#play-pause").label) == "取消"
        service.phase = "error"
        service.error = "该版本仅返回试听或不完整音频，未获取到可播放的完整版。"
        service.error_code = "incomplete_audio"
        app.render_state(service.status())
        content = str(app.query_one("#notice", Static).render())
        assert "完整音频" in content
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
        assert "暂无匹配" in str(app.query_one("#lyrics-status", Static).render())
        assert app.query_one("#lyrics-recovery").display


async def test_refresh_lyrics_while_playing_updates_display_without_changing_playback(
    paths, service, tracks
):
    class LyricsProvider:
        def lyrics(self, track):
            return "[00:01]新的歌词\n[00:05]下一行"

    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    service.provider = LyricsProvider()
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
        await service.lyric_task
        app.render_state(service.status())
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


async def test_space_always_controls_current_song_and_single_click_only_selects(
    paths, service, tracks
):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.search_tracks = tracks
    async with app.run_test(size=(123, 41)) as pilot:
        app.show_view("search")
        app.query_one("#results", DataTable).focus()
        await pilot.press("enter")
        await pilot.pause()
        await service.play_task
        app.poll()
        await pilot.pause()
        table = app.query_one("#results", DataTable)
        await pilot.click("#results", offset=(7, 2))
        await pilot.pause()
        assert table.cursor_row == 1
        assert service.current == tracks[0]
        await pilot.press("space")
        await pilot.pause()
        assert service.phase == "paused"
        assert service.current == tracks[0]
        await pilot.press("enter")
        await pilot.pause()
        await service.play_task
        assert service.current == tracks[1]
        await pilot.click("#results", offset=(7, 1), times=2)
        await pilot.pause()
        await service.play_task
        assert service.current == tracks[0]


async def test_narrow_terminal_can_open_lyrics_and_retry_failed_audio(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    service.queue, service.index = tracks, 1
    service.phase, service.error = "error", "网络请求失败"
    service.lyrics_status = "unavailable"
    async with app.run_test(size=(60, 24)) as pilot:
        await pilot.pause()
        app.poll()
        await pilot.pause()
        await pilot.press("v")
        await pilot.pause()
        assert app.query_one("#now").display
        assert not app.query_one("#collection").display
        assert app.query_one("#retry-audio").region.bottom <= 24
        await pilot.click("#retry-audio")
        await pilot.pause()
        await service.play_task
        assert service.current == tracks[1]
        assert service.phase == "playing"
        await pilot.press("escape")
        await pilot.pause()
        assert app.query_one("#collection").display


@pytest.mark.parametrize("size", [(123, 41), (80, 26), (60, 24)])
async def test_download_dialog_starts_pinned_track_and_background_task(
    paths, service, tracks, tmp_path, size
):
    from test_downloads import DownloadProvider

    service.provider = DownloadProvider()
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.search_tracks = tracks
    async with app.run_test(size=size) as pilot:
        app.show_view("search")
        app.query_one("#results", DataTable).move_cursor(row=1)
        app.action_download()
        await pilot.pause()
        assert "夜曲" in str(app.screen.query_one(".dialog-caption").render())
        app.screen.query_one("#download-directory", Input).value = str(tmp_path / "saved")
        await pilot.click("#download-quality")
        await pilot.press("down", "enter")
        await pilot.pause()
        assert app.screen.query_one("#download-quality", Select).value == "128k"
        await pilot.click("#download-start")
        await pilot.pause()
        await app.workers.wait_for_complete()
        await asyncio.gather(*service.downloads.tasks.values())
        job = service.downloads.status()[0]
        assert job["track"]["id"] == tracks[1].id
        assert job["quality"] == "128k"
        assert job["phase"] == "done"
        assert service.player.loads == []
        app.poll()
        await pilot.pause()
        app.action_downloads()
        await pilot.pause()
        assert "夜曲" in str(
            app.screen.query_one("#download-options").get_option_at_index(0).prompt
        )


async def test_song_actions_can_collect_without_changing_playback(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.store.add_to_playlist("通勤", tracks[0])
    app.search_tracks = tracks
    async with app.run_test(size=(100, 32)) as pilot:
        app.show_view("search")
        app.query_one("#results", DataTable).move_cursor(row=1)
        app.query_one("#results").focus()
        await pilot.press("x")
        await pilot.pause()
        menu = app.screen.query_one("#track-options")
        menu.highlighted = 3
        await pilot.press("enter")
        await pilot.pause()
        assert app.screen.__class__.__name__ == "FolderPicker"
        await pilot.click("#save")
        await pilot.pause()
        assert app.store.playlist("通勤") == tracks
        assert service.player.loads == []


async def test_download_survives_ui_exit(paths, service, tracks, tmp_path):
    import threading

    from test_downloads import DownloadProvider

    entered, release = threading.Event(), threading.Event()

    class Slow(DownloadProvider):
        def download(self, *args, **kwargs):
            entered.set()
            release.wait(2)
            return super().download(*args, **kwargs)

    service.provider = Slow()
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    try:
        async with app.run_test(size=(100, 32)) as pilot:
            app.control(
                "download",
                track=tracks[0].data(),
                directory=str(tmp_path / "saved"),
                quality="best",
            )
            await pilot.pause()
            await app.workers.wait_for_complete()
            assert await asyncio.to_thread(entered.wait, 1)
        release.set()
        await asyncio.gather(*service.downloads.tasks.values())
        assert service.downloads.status()[0]["phase"] == "done"
    finally:
        release.set()


async def test_loading_transport_can_cancel_without_late_playback(paths, service, tracks):
    import threading

    from conftest import FakeProvider

    entered, release = threading.Event(), threading.Event()

    class Slow(FakeProvider):
        def stream(self, track, quality):
            entered.set()
            release.wait(2)
            return super().stream(track, quality)

    service.provider = Slow()
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    try:
        async with app.run_test(size=(100, 32)) as pilot:
            await service.dispatch({"op": "play", "tracks": [tracks[0].data()]})
            app.state = service.status()
            app.render_state(app.state)
            assert str(app.query_one("#play-pause").label) == "取消"
            assert await asyncio.to_thread(entered.wait, 1)
            await pilot.click("#play-pause")
            await pilot.pause()
            release.set()
            await pilot.pause(0.05)
            assert service.phase == "stopped"
            assert service.player.loads == []
    finally:
        release.set()


async def test_quality_picker_retries_current_without_replacing_queue(paths, service, tracks):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    service.queue, service.index = tracks, 1
    service.phase, service.error = "error", "播放失败"
    async with app.run_test(size=(60, 24)) as pilot:
        app.state = service.status()
        app.render_state(app.state)
        app.action_now()
        await pilot.pause()
        await pilot.click("#retry-best")
        await pilot.pause()
        menu = app.screen.query_one("#quality-options")
        menu.highlighted = 3
        await pilot.press("enter")
        await pilot.pause()
        await service.play_task
        assert service.current == tracks[1]
        assert service.queue == tracks
        assert service.requested_quality == "128k"


@pytest.mark.parametrize("size", [(123, 41), (80, 26), (60, 24)])
async def test_quiet_player_rows_volume_and_help_remain_usable(paths, service, tracks, size):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    service.queue, service.index = tracks, 1
    service.phase = "paused"
    service.actual_quality = "320k"
    async with app.run_test(size=size) as pilot:
        await pilot.pause()
        current = app.query_one("#current-song")
        for identity in ("previous", "play-pause", "next", "play-mode", "volume-label"):
            button = app.query_one(f"#{identity}")
            assert button.region.y == current.region.y
            assert button.region.right <= size[0]
        assert app.query_one("#time-row").region.y == current.region.y + 1
        await pilot.click("#volume-label")
        await pilot.pause()
        app.screen.query_one("#volume-input", Input).value = "37"
        await pilot.press("enter")
        await pilot.pause()
        assert service.settings.values["volume"] == 37
        await pilot.press("plus")
        await pilot.pause()
        assert service.settings.values["volume"] == 42
        await pilot.click("#help-open")
        await pilot.pause()
        assert app.screen.__class__.__name__ == "ShortcutHelp"
        assert app.screen.query_one("#help-close").region.bottom <= size[1]
        await pilot.press("escape")
        await pilot.pause()


async def test_current_song_menu_targets_playing_track_and_folder_management_menu(
    paths, service, tracks
):
    app = MusicApp(paths=paths)
    app.client = UIClient(service)
    app.store.add_to_playlist("通勤", tracks[0])
    service.queue, service.index = tracks, 1
    service.phase = "paused"
    async with app.run_test(size=(123, 41)) as pilot:
        app.show_view("playlist:通勤")
        await pilot.pause()
        await pilot.click("#now-menu")
        await pilot.pause()
        options = app.screen.query_one("#track-options")
        assert options.get_option_at_index(4).id == "lyrics"
        options.highlighted = 2
        await pilot.press("enter")
        await pilot.pause()
        assert app.store.is_favorite(tracks[1].id)
        assert not app.store.is_favorite(tracks[0].id)
        await pilot.click("#collection-menu")
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        assert app.screen.__class__.__name__ == "NameDialog"
        app.screen.query_one("#folder-name", Input).value = "路上"
        await pilot.press("enter")
        await pilot.pause()
        assert app.active_view == "playlist:路上"
        assert app.store.playlist("路上") == [tracks[0]]
