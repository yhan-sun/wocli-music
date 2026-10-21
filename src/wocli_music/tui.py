import asyncio
import random
from pathlib import Path

from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import (
    Button,
    DataTable,
    Input,
    OptionList,
    Select,
    Static,
)
from textual.widgets.option_list import Option

from .client import Client
from .config import Paths, Settings
from .dialogs import (
    DOWNLOAD_PHASES,
    MODE_LABELS,
    ConfirmDelete,
    DownloadDialog,
    DownloadList,
    FolderActions,
    FolderPicker,
    LibraryPicker,
    ModePicker,
    NameDialog,
    QualityPicker,
    ShortcutHelp,
    TrackActions,
    VolumeDialog,
)
from .lyrics import current_line, parse_lrc
from .models import MusicError, Track, clock
from .provider import KuwoProvider
from .store import Store
from .widgets import SeekBar, SongTable


class MusicApp(App):
    """A keyboard-first music library; playback survives closing the interface."""

    TITLE = "wocli"
    CSS = """
    * { scrollbar-background: #18251c; scrollbar-color: #405937;
        scrollbar-color-hover: #789766; scrollbar-color-active: #bddb92; }
    Screen { background: #121b16; color: #dce3d2; }
    #masthead { height: 3; padding: 0 2; align-vertical: middle; }
    #wordmark { width: 1fr; color: #c2dc9c; text-style: bold; padding-top: 1; }
    Button { min-width: 5; height: 1; padding: 0; border: none; background: #26382a; color: #dce3d2; }
    Button:hover { background: #3b5233; }
    Button:focus { text-style: bold; background: #405b35; }
    #library-open { display: none; min-width: 6; margin-right: 1; }
    #search-open { width: 10; margin: 1 1 0 0; }
    #library-open, #now-open, #download-jobs { margin: 1 1 0 0; }
    #now-open { width: 10; }
    #download-jobs { width: 9; }
    #quality { width: 14; height: 1; margin-top: 1; margin-right: 1; }
    Select { border: none; }
    SelectCurrent { background: #1c2b20; border: none; padding: 0 1; }
    Input { border: tall #425a3b; background: #1b2a1e; }
    Input:focus { border: tall #bddb92; }
    #body { height: 1fr; padding: 0 2; }
    #sidebar { width: 18; padding: 1 1 0 0; border-right: solid #29392d; }
    .eyebrow { height: 2; color: #81967a; }
    #library { height: 1fr; padding: 0; border: none; background: #121b16; }
    #library > .option-list--option { padding: 0 1; }
    #library > .option-list--option-highlighted { background: #30452c; color: #d8ebba; }
    #library > .option-list--option-disabled { color: #72836b; }
    #library-heading { height: 2; }
    #library-heading Static { width: 1fr; }
    #new-folder { min-width: 3; width: 3; background: transparent; color: #8fa482; }
    #collection-menu { min-width: 3; width: 3; margin-left: 1; background: transparent; }
    #collection { width: 1fr; padding: 1 2 0 1; min-width: 26; }
    #view-heading { height: 2; }
    #view-heading Button { margin-left: 1; }
    #view-title { width: 1fr; text-overflow: ellipsis; text-wrap: nowrap; color: #d9e9c4; text-style: bold; }
    #view-count { width: auto; color: #829b87; }
    #list-status { height: 1; display: none; color: #829b87; margin-bottom: 1; }
    #searchbar { height: 3; display: none; margin-top: 1; }
    #search { width: 1fr; }
    #search-button { width: 8; margin: 1 0 0 1; }
    #now-heading Static { width: 1fr; }
    #now-menu { min-width: 6; background: transparent; color: #8fa482; }
    #lyrics-recovery { height: auto; max-height: 2; margin-top: 1; }
    #lyrics-status { width: 1fr; height: auto; max-height: 2; color: #8fa482; }
    #retry-lyrics { min-width: 6; }
    #notice { height: auto; max-height: 4; color: #dfa38a; margin-top: 1; }
    #retry-actions { height: 2; padding-top: 1; }
    #retry-actions Button { margin-right: 1; }
    #now-close { display: none; }
    .lyrics-focused #collection, .lyrics-focused #sidebar { display: none; }
    .lyrics-focused.has-track #now { display: block; width: 1fr; max-width: 100%; }
    .lyrics-focused #now-close { display: block; }
    #current-song { width: 1fr; height: 1; color: #d4e6bd; margin-right: 2; text-overflow: ellipsis; text-wrap: nowrap; }
    #player-row { height: 1; }
    #audio-quality { width: auto; min-width: 6; color: #829b87; margin-left: 2; text-align: right; }
    #controls { width: auto; height: 1; }
    #play-all { background: transparent; color: #bddb92; }
    #track-menu { background: transparent; color: #8fa482; }
    #help-open { min-width: 3; width: 3; margin-top: 1; background: transparent; color: #8fa482; }
    #results { height: 1fr; background: #121b16; }
    DataTable > .datatable--header { background: #1e2c20; color: #8fa482; text-style: none; }
    DataTable > .datatable--cursor { background: #2b3c27; color: #e4efcc; }
    DataTable:focus > .datatable--cursor { background: #3b542e; color: #edf5d9; }
    #empty { height: 1fr; align: center middle; padding: 1; }
    #empty-search { width: 8; background: transparent; }
    #paging { height: 3; align: center middle; }
    #paging Button { height: 1; margin: 1 1; }
    #page { width: auto; margin: 1 1; color: #8fa482; }
    #now { display: none; width: 38%; min-width: 30; max-width: 46; background: #121b16; border-left: solid #29392d; padding: 1 2; }
    .has-track #now { display: block; }
    #lyrics { height: 1fr; padding-top: 1; content-align: center middle; }
    #transport { height: 3; padding: 1 2 0 2; background: #1a281e; }
    #time-row { height: 1; }
    #elapsed, #duration { width: 6; color: #a8be96; }
    #timeline { width: 1fr; padding: 0 1; }
    #controls Button { height: 1; margin-right: 1; }
    #previous, #next { width: 3; min-width: 3; background: transparent; }
    #play-pause { width: 6; min-width: 6; }
    #play-pause { background: #bddb92; color: #152014; }
    #favorite { min-width: 3; }
    #play-mode { min-width: 13; }
    #volume-label { width: 6; min-width: 5; color: #93aa87; background: transparent; }
    .narrow #now { display: none; }
    .narrow.lyrics-focused.has-track #now { display: block; width: 1fr; max-width: 100%; }
    .narrow #collection { padding-right: 0; }
    .compact #lyrics { padding-top: 0; }
    .compact #sidebar { width: 18; }
    .tiny #library-open { display: block; }
    .tiny #sidebar, .tiny #quality, .tiny #favorite { display: none; }
    .tiny #body, .tiny #masthead, .tiny #transport { padding-left: 1; padding-right: 1; }
    """
    BINDINGS = [
        Binding("question_mark", "help", "帮助", show=False),
        Binding("plus,equals", "volume_up", "音量 +", show=False),
        Binding("minus", "volume_down", "音量 -", show=False),
        Binding("ctrl+f,slash", "search", "搜索"),
        Binding("space", "play_or_toggle", "播放/暂停"),
        Binding("x", "track_menu", "歌曲操作"),
        Binding("d", "download", "下载", show=False),
        Binding("v", "now", "播放/歌词", show=False),
        Binding("ctrl+r", "retry_audio", "重试音源", show=False),
        Binding("f", "favorite", "喜欢", show=False),
        Binding("c", "collect", "收藏到…", show=False),
        Binding("m", "mode", "播放方式"),
        Binding("b", "library", "音乐库", show=False),
        Binding("n", "new_folder", "新建夹", show=False),
        Binding("r", "rename_folder", "重命名夹", show=False),
        Binding("delete,backspace", "remove_track", "移出", show=False),
        Binding("a", "enqueue", "加入队列", show=False),
        Binding("l", "refresh_lyrics", "刷新歌词", show=False),
        Binding("ctrl+right", "next", "下一首", show=False),
        Binding("ctrl+left", "prev", "上一首", show=False),
        Binding("right", "seek_forward", "+5s", show=False),
        Binding("left", "seek_back", "−5s", show=False),
        Binding("escape", "focus_list", "返回", show=False),
        Binding("q,ctrl+q,ctrl+c", "quit", "退出界面"),
    ]

    def __init__(self, *, direct=False, paths=None):
        super().__init__()
        self.paths = paths or Paths.discover()
        self.settings = Settings(self.paths)
        self.store = Store(self.paths)
        self.provider = KuwoProvider(direct=direct or self.settings.values["direct"])
        self.client = Client(self.paths)
        self.rows = {"results": []}
        self.search_tracks = []
        self.active_view = self.store.load("ui.view", "favorites")
        self.previous_view = self.active_view
        self.state = None
        self.keyword, self.page, self.total = "", 0, 0
        self.search_serial = 0
        self.search_status = ""
        self.last_queue = None
        self.last_track = None
        self.last_lyric_text = None
        self.last_lyric_index = None
        self.lyric_lines = []
        self.favorite_ids = set()
        self.navigation_signature = None
        self.table_signature = None
        self.table_width = None
        self.lyrics_focused = False
        self.last_downloads = []

    def compose(self) -> ComposeResult:
        with Horizontal(id="masthead"):
            yield Static("wocli", id="wordmark")
            yield Button("曲库", id="library-open")
            yield Button("搜索", id="search-open")
            yield Button("歌词", id="now-open")
            yield Button("下载", id="download-jobs")
            yield Select(
                [
                    ("MP3 · 320k", "320k"),
                    ("MP3 · 128k", "128k"),
                    ("FLAC · 无损", "flac"),
                    ("最佳可用", "best"),
                ],
                value=self.settings.values["quality"],
                allow_blank=False,
                compact=True,
                id="quality",
            )
            yield Button("?", id="help-open")
        with Horizontal(id="body"):
            with Vertical(id="sidebar"):
                with Horizontal(id="library-heading"):
                    yield Static("曲库")
                    yield Button("+", id="new-folder")
                yield OptionList(id="library")
            with Vertical(id="collection"):
                with Horizontal(id="view-heading"):
                    yield Static("我喜欢", id="view-title")
                    yield Static("", id="view-count")
                    yield Button("播放全部", id="play-all")
                    yield Button("操作", id="track-menu")
                    yield Button("⋯", id="collection-menu")
                yield Static("", id="list-status")
                with Horizontal(id="searchbar"):
                    yield Input(placeholder="歌名 / 歌手", id="search")
                    yield Button("搜索", id="search-button")
                yield SongTable(id="results", cursor_type="row")
                with Vertical(id="empty"):
                    yield Button("搜索", id="empty-search")
                with Horizontal(id="paging"):
                    yield Button("上一页", id="page-prev")
                    yield Static("—", id="page")
                    yield Button("下一页", id="page-next")
            with Vertical(id="now"):
                with Horizontal(classes="eyebrow", id="now-heading"):
                    yield Static("歌词")
                    yield Button("操作", id="now-menu")
                    yield Button("返回", id="now-close")
                yield Static("", id="notice")
                with Horizontal(id="retry-actions"):
                    yield Button("重试音源", id="retry-audio")
                    yield Button("换音质", id="retry-best")
                    yield Button("跳过", id="skip-error")
                with Horizontal(id="lyrics-recovery"):
                    yield Static("", id="lyrics-status")
                    yield Button("重试", id="retry-lyrics")
                yield Static("", id="lyrics")
        with Vertical(id="transport"):
            with Horizontal(id="player-row"):
                yield Static("", id="current-song")
                with Horizontal(id="controls"):
                    yield Button("‹", id="previous")
                    yield Button("播放", id="play-pause")
                    yield Button("›", id="next")
                    yield Button("♡", id="favorite")
                    yield Button(MODE_LABELS[self.settings.values["mode"]] + " ▾", id="play-mode")
                    yield Button(f"{self.settings.values['volume']}%", id="volume-label")
            with Horizontal(id="time-row"):
                yield Static("00:00", id="elapsed")
                yield SeekBar(id="timeline")
                yield Static("00:00", id="duration")
                yield Static("", id="audio-quality")

    def on_mount(self):
        self.resize_tables()
        self.refresh_library()
        self.show_view(self.active_view)
        self.query_one("#results").focus()
        self.set_interval(0.5, self.poll)
        self.set_interval(2, self.refresh_library)
        self.poll()
        for identity, tooltip in {
            "track-menu": "所选歌曲：播放、队列、收藏、下载与移出 (x)",
            "favorite": "喜欢当前播放的歌曲",
            "retry-best": "选择其他音质并重新解析当前歌曲",
            "now-open": "展开当前播放与歌词 (v)",
            "now-menu": "当前歌曲：喜欢、收藏、下载与刷新歌词",
            "new-folder": "新建收藏夹 (n)",
            "collection-menu": "收藏夹管理",
            "previous": "上一首 (Ctrl+←)",
            "next": "下一首 (Ctrl+→)",
            "volume-label": "调整音量 (+ / -)",
            "help-open": "快捷键 (?)",
        }.items():
            self.query_one(f"#{identity}").tooltip = tooltip
        self.render_context()
        self.on_resize()

    def on_resize(self):
        self.screen_stack[0].set_class(self.size.width < 120, "narrow")
        self.screen_stack[0].set_class(self.size.width < 100, "compact")
        self.screen_stack[0].set_class(self.size.width < 68, "tiny")
        self.call_after_refresh(self.resize_tables)

    def resize_tables(self):
        if not self.query("#results"):
            return
        table = self.query_one("#results", DataTable)
        if self.table_width == table.size.width:
            return
        self.table_width = table.size.width
        available = max(16, table.size.width - 18)
        title = max(10, int(available * 0.64))
        cursor = table.cursor_row
        table.clear(columns=True)
        self.table_signature = None
        for label, width in (
            ("", 3),
            ("歌曲", title),
            ("歌手", max(6, available - title)),
            ("时长", 5),
        ):
            table.add_column(label, width=width)
        self.fill_table("results", self.rows["results"])
        if table.row_count:
            table.move_cursor(row=min(cursor, table.row_count - 1))

    def on_unmount(self):
        self.store.close()

    def check_action(self, action, parameters):
        if isinstance(self.screen, ModalScreen):
            return False
        if isinstance(self.focused, Input) and action not in ("search", "focus_list"):
            return False
        return True

    def on_key(self, event):
        if event.key in ("ctrl+q", "ctrl+c"):
            event.stop()
            self.exit()

    def fill_table(self, name, tracks):
        table = self.query_one(f"#{name}", DataTable)
        current = (self.state or {}).get("track") or {}
        signature = (
            [track.data() for track in tracks],
            current.get("id"),
            self.favorite_ids.copy(),
        )
        if signature == self.table_signature:
            return
        self.table_signature = signature
        previous = self.rows.get(name, [])
        cursor = table.cursor_row
        identity = previous[cursor].id if cursor < len(previous) else None
        table.clear()
        self.rows[name] = tracks
        current = (self.state or {}).get("track") or {}
        for i, track in enumerate(tracks):
            marker = (
                "▶"
                if track.id == current.get("id")
                else "♥"
                if track.id in self.favorite_ids
                else str(i + 1)
            )
            table.add_row(
                marker, Text(track.title), Text(track.artist), clock(track.duration), key=str(i)
            )
        if tracks:
            row = (
                cursor
                if cursor < len(tracks) and tracks[cursor].id == identity
                else next(
                    (i for i, track in enumerate(tracks) if track.id == identity),
                    min(cursor, len(tracks) - 1),
                )
            )
            table.move_cursor(row=row)

    def queue_tracks(self):
        queue = (
            self.state["queue"]
            if self.state
            else self.store.load("queue", {"tracks": []})["tracks"]
        )
        return [Track.from_data(track) for track in queue]

    def refresh_library(self):
        if not self.query("#library"):
            return
        self.resize_tables()
        favorites, folders = self.store.favorites(), self.store.playlists()
        self.favorite_ids = {track.id for track in favorites}
        options = [
            Option(f"♥ 我喜欢   {len(favorites)}", id="favorites"),
            Option(f"≡ 播放队列   {len(self.queue_tracks())}", id="queue"),
            Option("◷ 最近播放", id="history"),
            None,
            Option("我的收藏夹", id="folder-label", disabled=True),
        ]
        options += [
            Option(Text(f"  {folder['name']}  {folder['count']}"), id="playlist:" + folder["name"])
            for folder in folders
        ]
        signature = [(option.id, str(option.prompt)) for option in options if option is not None]
        if signature != self.navigation_signature:
            nav = self.query_one("#library", OptionList)
            highlighted = (
                nav.get_option_at_index(nav.highlighted).id
                if nav.highlighted is not None
                else self.active_view
            )
            nav.clear_options().add_options(options)
            self.navigation_signature = signature
            self.highlight_view(highlighted if nav.has_focus else self.active_view)
        self.show_view(self.active_view, persist=False)

    def highlight_view(self, view=None):
        nav = self.query_one("#library", OptionList)
        for i in range(nav.option_count):
            if nav.get_option_at_index(i).id == (view or self.active_view):
                nav.highlighted = i
                break

    def show_view(self, view, *, persist=True):
        titles = {
            "favorites": "我喜欢",
            "queue": "播放队列",
            "history": "最近播放",
            "search": "搜索音乐",
        }
        try:
            tracks = (
                self.store.favorites()
                if view == "favorites"
                else self.queue_tracks()
                if view == "queue"
                else self.store.history()
                if view == "history"
                else self.search_tracks
                if view == "search"
                else self.store.playlist(view.removeprefix("playlist:"))
            )
        except MusicError:
            view, tracks = "favorites", self.store.favorites()
        changed = view != self.active_view
        self.active_view = view
        if persist and view != "search":
            self.store.save("ui.view", view)
        self.query_one("#view-title", Static).update(
            Text(titles.get(view, view.removeprefix("playlist:")))
        )
        count = self.total if view == "search" else len(tracks)
        self.query_one("#view-count", Static).update(f"{count:,} 首")
        status = self.search_status if view == "search" else ""
        self.query_one("#list-status", Static).update(status)
        self.query_one("#list-status").display = bool(status)
        self.query_one("#searchbar").display = view == "search"
        self.query_one("#paging").display = view == "search" and bool(tracks)
        self.query_one("#page", Static).update(
            f"{self.page + 1} / {max(1, (self.total + 19) // 20)}"
        )
        self.query_one("#page-prev", Button).disabled = self.page == 0
        self.query_one("#page-next", Button).disabled = (self.page + 1) * 20 >= self.total
        self.query_one("#collection-menu").display = view.startswith("playlist:")
        for identity in ("play-all",):
            self.query_one(f"#{identity}", Button).disabled = not tracks
        self.fill_table("results", tracks)
        self.query_one("#results").display = bool(tracks)
        self.query_one("#empty").display = not tracks
        self.query_one("#empty-search").display = view != "search"
        if persist or changed:
            self.highlight_view()
        if changed:
            self.query_one("#results", DataTable).move_cursor(row=0)
        self.call_after_refresh(self.resize_tables)
        self.render_context()

    @on(OptionList.OptionSelected, "#library")
    def library_selected(self, event):
        if event.option.id and event.option.id != "folder-label":
            self.show_view(event.option.id)
            self.query_one("#results").focus()

    def selected(self) -> Track | None:
        tracks = self.rows["results"]
        index = self.query_one("#results", DataTable).cursor_row
        return tracks[index] if 0 <= index < len(tracks) else None

    @on(Input.Submitted, "#search")
    def search_submitted(self):
        self.start_search()

    @on(Select.Changed, "#quality")
    def quality_changed(self, event):
        if event.value != Select.BLANK and event.value != self.settings.values["quality"]:
            self.save_quality(str(event.value))

    @work(exclusive=True, group="quality")
    async def save_quality(self, value):
        try:
            state = await self.client.request("status", autostart=False)
            if state:
                await self.client.request("config", key="quality", value=value)
            else:
                self.settings.set("quality", value)
            self.settings.values["quality"] = value
        except MusicError as error:
            self.notify(str(error), severity="error", markup=False)

    def start_search(self):
        keyword = self.query_one("#search", Input).value.strip()
        if keyword:
            self.keyword, self.page = keyword, 0
            self.show_view("search")
            self.fetch_results()

    @work(exclusive=True, group="search")
    async def fetch_results(self):
        self.search_serial += 1
        serial, keyword, page = self.search_serial, self.keyword, self.page
        self.search_status = "搜索中…"
        self.show_view("search", persist=False)
        try:
            total, tracks = await asyncio.to_thread(self.provider.search, keyword, page, 20)
            if serial != self.search_serial:
                return
            self.total, self.search_tracks = total, tracks
            self.store.remember(tracks)
            self.search_status = "" if tracks else "无结果"
            # Keep results when a modal is open; the library renders them on return.
            if self.active_view == "search" and self.query("#results"):
                self.show_view("search", persist=False)
                if tracks:
                    self.query_one("#results", DataTable).move_cursor(row=0)
                    self.query_one("#results").focus()
        except MusicError as error:
            self.search_status = Text(str(error), style="#e2a194")
            if self.active_view == "search" and self.query("#results"):
                self.show_view("search", persist=False)

    @on(DataTable.RowSelected, "#results")
    def row_selected(self):
        self.play_selection()

    @work(exclusive=True, group="play")
    async def play_selection(self, *, all_tracks=False, shuffle=False):
        tracks = list(self.rows["results"])
        index = 0 if all_tracks else self.query_one("#results", DataTable).cursor_row
        if not tracks:
            return
        if shuffle:
            index = random.randrange(len(tracks))
        queue_view = self.active_view == "queue"
        quality = self.query_one("#quality", Select).value
        try:
            if shuffle:
                await self.client.request("mode", value="shuffle")
            self.state = await self.client.request(
                "play",
                index=index,
                quality=quality,
                **({} if queue_view else {"tracks": [track.data() for track in tracks]}),
            )
            self.render_state(self.state)
            self.poll()
        except MusicError as error:
            self.notify(str(error), severity="error", markup=False)

    def action_library(self):
        def choose(view):
            if view == "create-folder":
                self.action_new_folder()
            elif view:
                self.show_view(view)
                self.query_one("#results").focus()

        self.push_screen(LibraryPicker(self.store.playlists()), choose)

    def action_search(self):
        if self.lyrics_focused:
            self.action_now()
        if self.active_view != "search":
            self.previous_view = self.active_view
        self.show_view("search")
        self.query_one("#search").focus()

    def action_focus_list(self):
        if self.lyrics_focused:
            self.action_now()
            return
        if self.active_view == "search":
            self.show_view(self.previous_view)
        self.query_one("#results").focus()

    def action_collect(self, track=None):
        track = (
            track
            or self.selected()
            or (
                Track.from_data(self.state["track"]) if self.state and self.state["track"] else None
            )
        )
        if not track:
            return

        def collect(result):
            if result:
                name, _ = result
                self.store.add_to_playlist(name, track)
                self.store.save("ui.last_folder", name)
                self.refresh_library()

        self.push_screen(
            FolderPicker(
                track,
                self.store.playlists(),
                self.store.playlists_for_track(track.id),
                self.store.load("ui.last_folder"),
            ),
            collect,
        )

    def action_new_folder(self):
        def create(name):
            if name:
                self.store.create_playlist(name)
                self.refresh_library()
                self.show_view("playlist:" + name)

        self.push_screen(
            NameDialog(existing=[folder["name"] for folder in self.store.playlists()]), create
        )

    def action_rename_folder(self):
        if not self.active_view.startswith("playlist:"):
            return
        old = self.active_view.removeprefix("playlist:")

        def rename(name):
            if name and name != old:
                self.store.rename_playlist(old, name)
                self.active_view = "playlist:" + name
                self.store.save("ui.view", self.active_view)
                if self.store.load("ui.last_folder") == old:
                    self.store.save("ui.last_folder", name)
                self.refresh_library()

        self.push_screen(
            NameDialog("重命名收藏夹", old, [folder["name"] for folder in self.store.playlists()]),
            rename,
        )

    def delete_folder(self):
        if not self.active_view.startswith("playlist:"):
            return
        name = self.active_view.removeprefix("playlist:")

        def remove(confirmed):
            if confirmed:
                self.store.delete_playlist(name)
                self.show_view("favorites")
                self.refresh_library()

        self.push_screen(ConfirmDelete(name), remove)

    def action_remove_track(self):
        track = self.selected()
        if not track:
            return
        if self.active_view == "favorites":
            if self.store.is_favorite(track.id):
                self.store.favorite(track)
        elif self.active_view.startswith("playlist:"):
            self.store.remove_from_playlist(self.active_view.removeprefix("playlist:"), track.id)
        elif self.active_view == "queue":
            self.control("queue_remove", index=self.query_one("#results", DataTable).cursor_row)
            return
        self.refresh_library()

    def action_help(self):
        self.push_screen(ShortcutHelp())

    def action_folder_menu(self):
        if not self.active_view.startswith("playlist:"):
            return

        def choose(action):
            if action == "rename":
                self.action_rename_folder()
            elif action == "delete":
                self.delete_folder()

        self.push_screen(FolderActions(self.active_view.removeprefix("playlist:")), choose)

    def action_volume(self):
        value = self.state["volume"] if self.state else self.settings.values["volume"]
        self.push_screen(
            VolumeDialog(value),
            lambda value: self.control("volume", value=value) if value is not None else None,
        )

    def action_volume_up(self):
        value = self.state["volume"] if self.state else self.settings.values["volume"]
        self.control("volume", value=min(100, value + 5))

    def action_volume_down(self):
        value = self.state["volume"] if self.state else self.settings.values["volume"]
        self.control("volume", value=max(0, value - 5))

    def action_mode(self):
        mode = self.state["mode"] if self.state else self.settings.values["mode"]
        self.push_screen(
            ModePicker(mode), lambda value: self.control("mode", value=value) if value else None
        )

    @on(Button.Pressed)
    def button_pressed(self, event):
        actions = {
            "track-menu": self.action_track_menu,
            "now-menu": lambda: self.action_track_menu(current=True),
            "collection-menu": self.action_folder_menu,
            "help-open": self.action_help,
            "volume-label": self.action_volume,
            "download-jobs": self.action_downloads,
            "now-open": self.action_now,
            "now-close": self.action_now,
            "retry-audio": self.action_retry_audio,
            "retry-best": self.action_retry_quality,
            "retry-lyrics": self.action_refresh_lyrics,
            "skip-error": self.action_next,
            "library-open": self.action_library,
            "search-open": self.action_search,
            "empty-search": self.action_search,
            "search-button": self.start_search,
            "play-pause": self.action_play_or_toggle,
            "new-folder": self.action_new_folder,
            "play-mode": self.action_mode,
        }
        identity = event.button.id
        if identity in actions:
            actions[identity]()
        elif identity == "play-all":
            self.play_selection(all_tracks=True)
        elif identity == "play-shuffled":
            self.play_selection(all_tracks=True, shuffle=True)
        elif identity in ("page-prev", "page-next"):
            self.page += -1 if identity == "page-prev" else 1
            self.fetch_results()
        elif identity in ("previous", "next"):
            self.control("prev" if identity == "previous" else "next")
        elif identity == "favorite":
            self.toggle_favorite(
                Track.from_data(self.state["track"]) if self.state and self.state["track"] else None
            )

    def toggle_favorite(self, track):
        if not track:
            return
        self.store.favorite(track)
        self.refresh_library()
        if self.state:
            self.render_state(self.state)

    def action_favorite(self):
        self.toggle_favorite(self.context_track())

    def action_enqueue(self):
        if track := self.context_track():
            self.control("queue_add", tracks=[track.data()])

    def action_play_selection(self):
        self.play_selection()

    def action_play_or_toggle(self):
        if self.state and self.state["phase"] == "loading":
            self.control("stop")
        elif self.state and self.state["phase"] in ("playing", "paused"):
            self.control("toggle")
        elif self.state and self.state.get("track"):
            self.action_retry_audio()
        else:
            self.play_selection()

    def action_next(self):
        self.control("next")

    def action_prev(self):
        self.control("prev")

    @on(SeekBar.SeekRequested)
    def timeline_seek(self, event):
        self.control("seek_to", position=event.fraction * event.duration, track_id=event.track_id)

    def action_seek_forward(self):
        self.control("seek", seconds=5)

    def action_seek_back(self):
        self.control("seek", seconds=-5)

    @on(DataTable.RowHighlighted, "#results")
    def selection_changed(self):
        self.render_context()

    def render_context(self):
        if not self.query("#results"):
            return
        track = self.selected()
        self.query_one("#track-menu", Button).disabled = not track
        if self.state is None:
            self.query_one("#notice").display = False
            self.query_one("#retry-actions").display = False
            self.query_one("#lyrics-recovery").display = False
            for identity in (
                "retry-lyrics",
                "now-menu",
                "now-open",
                "previous",
                "next",
                "favorite",
            ):
                self.query_one(f"#{identity}", Button).disabled = True

    def render_now_status(self, state):
        track = state.get("track")
        phase = state["phase"]
        label = {
            "playing": "播放中",
            "paused": "已暂停",
            "loading": "正在重试" if state.get("retry_attempt", 1) > 1 else "解析中",
            "error": "播放失败",
            "stopped": "已停止",
        }[phase]
        self.query_one("#current-song", Static).update(
            Text(f"{track['title']} · {track['artist']}" if track else "")
        )
        self.query_one("#current-song").tooltip = (
            f"{label} · {track['title']} / {track['artist']}" if track else None
        )
        self.query_one("#now-open", Button).disabled = not track
        self.query_one("#play-pause", Button).disabled = False
        if phase == "loading":
            self.query_one("#play-pause", Button).label = "取消"
        if phase == "loading":
            self.query_one("#audio-quality", Static).update(
                "重试中" if state.get("retry_attempt", 1) > 1 else "解析中"
            )
        elif phase == "error":
            self.query_one("#audio-quality", Static).update("失败")
        self.query_one("#notice").display = phase == "error"
        self.query_one("#retry-actions").display = phase == "error"
        if phase == "error":
            self.query_one("#notice", Static).update(
                Text(
                    "未获得完整音频，可重试、换音质或跳过。"
                    if state.get("error_code") == "incomplete_audio"
                    else state.get("error", "播放失败").split("。", 1)[0]
                )
            )
        for identity in (
            "retry-audio",
            "retry-best",
            "now-menu",
            "previous",
            "next",
            "favorite",
        ):
            self.query_one(f"#{identity}", Button).disabled = not track or (
                identity.startswith("retry") and phase == "loading"
            )
        loading = state.get("lyrics_status") == "loading"
        self.query_one("#retry-lyrics", Button).disabled = not track or loading
        self.query_one("#lyrics-recovery").display = (
            bool(track)
            and not loading
            and bool(state.get("lyrics_error") or state.get("lyrics_status") == "unavailable")
        )
        self.query_one("#lyrics-status", Static).update(state.get("lyrics_error") or "暂无匹配歌词")
        jobs = state.get("downloads", [])
        active = [
            job for job in jobs if job["phase"] in ("queued", "resolving", "downloading", "lyrics")
        ]
        self.query_one("#download-jobs", Button).label = f"下载 {len(active)}" if active else "下载"
        if active:
            job = next((job for job in active if job["phase"] != "queued"), active[0])
            detail = (
                f" {job['received'] / max(1, job['total']):.0%}"
                if job["phase"] == "downloading"
                else ""
            )
            description = f"{job['track']['title']} · {DOWNLOAD_PHASES[job['phase']]}{detail}"
        elif jobs:
            job = jobs[-1]
            description = f"{DOWNLOAD_PHASES[job['phase']]} · {job.get('error') or job.get('warning') or job.get('path')}"
        else:
            description = "查看下载任务、进度和保存位置"
        self.query_one("#download-jobs").tooltip = description

    def action_now(self):
        if not (self.state or {}).get("track"):
            return
        self.lyrics_focused = not self.lyrics_focused
        self.screen_stack[0].set_class(self.lyrics_focused, "lyrics-focused")
        self.query_one("#now-open", Button).label = "返回曲库" if self.lyrics_focused else "歌词"
        self.call_after_refresh(self.resize_tables)

    def action_retry_quality(self):
        self.push_screen(
            QualityPicker(), lambda value: self.action_retry_audio(quality=value) if value else None
        )

    def action_retry_audio(self, quality=None):
        if not self.state or not self.state.get("track"):
            return
        if self.state["phase"] == "loading":
            return
        self.control("retry", **({"quality": quality} if quality else {}))

    def context_track(self):
        if self.lyrics_focused:
            current = (self.state or {}).get("track")
            return Track.from_data(current) if current else None
        return self.selected()

    def action_track_menu(self, *, current=False):
        current_context = current or self.lyrics_focused
        data = (self.state or {}).get("track")
        track = Track.from_data(data) if current_context and data else self.context_track()
        if not track:
            return
        index = self.query_one("#results", DataTable).cursor_row
        removable = not current_context and (
            self.active_view in ("favorites", "queue") or self.active_view.startswith("playlist:")
        )

        def choose(action):
            if action == "lyrics":
                self.action_refresh_lyrics()
            elif action == "play" and current_context:
                self.control("retry")
            elif action == "play":
                self.query_one("#results", DataTable).move_cursor(row=index)
                self.play_selection()
            elif action == "enqueue":
                self.control("queue_add", tracks=[track.data()])
            elif action == "like":
                self.toggle_favorite(track)
            elif action == "collect":
                self.action_collect(track)
            elif action == "download":
                self.action_download(track=track)
            elif action == "remove":
                self.query_one("#results", DataTable).move_cursor(row=index)
                self.action_remove_track()

        self.push_screen(
            TrackActions(
                track,
                liked=self.store.is_favorite(track.id),
                removable=removable,
                current=current_context,
            ),
            choose,
        )

    def action_download(self, *, track=None, current=False):
        track = track or (
            Track.from_data(self.state["track"])
            if current and self.state and self.state.get("track")
            else self.context_track()
        )
        if not track:
            return
        if track.path:
            self.notify("这首歌已在本地，无需下载。")
            return

        def start(options):
            if options:
                self.store.save("ui.download_directory", options["directory"])
                self.control("download", track=track.data(), **options)

        directory = self.store.load(
            "ui.download_directory", str(Path.home() / "Music" / "wocli-music")
        )
        self.push_screen(DownloadDialog(track, directory, self.settings.values["quality"]), start)

    def action_downloads(self):
        def manage(result):
            if result:
                operation, identity = result
                self.control(operation, id=identity)

        self.push_screen(DownloadList(lambda: (self.state or {}).get("downloads", [])), manage)

    @work(exclusive=True, group="poll")
    async def poll(self):
        try:
            state = await self.client.request("status", autostart=False)
            if state:
                self.state = state
                self.render_state(state)
            elif self.query("#transport"):
                if self.state:
                    self.state = None
                    self.query_one("#audio-quality", Static).update("")
                    self.query_one("#play-pause", Button).label = "播放"
                    self.query_one("#current-song", Static).update("")
                    self.query_one("#elapsed", Static).update("00:00")
                    self.query_one("#duration", Static).update("00:00")
                    self.query_one("#timeline", SeekBar).set_state(0, 0, None, False)
                    self.screen_stack[0].remove_class("has-track", "lyrics-focused")
                    self.lyrics_focused = False
                    self.last_track = None
                    self.query_one("#now-open", Button).label = "歌词"
                    self.render_context()
        except MusicError:
            pass

    def render_state(self, state):
        if not self.query("#transport"):
            return
        state = dict(state)
        if state.get("track"):
            cached = self.store.lyric(state["track"]["id"])
            if cached and parse_lrc(cached):
                state["lyrics"] = cached
                if state.get("lyrics_status") != "loading":
                    state["lyrics_status"] = "ready"
        track = state["track"]
        if track != self.last_track:
            self.last_track = track
            self.screen_stack[0].set_class(bool(track), "has-track")
            self.last_lyric_index = None
            self.refresh_library()
        self.query_one("#elapsed", Static).update(clock(state["position"]))
        self.query_one("#duration", Static).update(clock(state["duration"]))
        self.query_one("#timeline", SeekBar).set_state(
            state["position"],
            state["duration"],
            track["id"] if track else None,
            state["phase"] in ("playing", "paused") and bool(track),
        )
        self.query_one("#play-pause", Button).label = (
            "暂停"
            if state["phase"] == "playing"
            else "重试"
            if state["phase"] == "error"
            else "播放"
        )
        self.query_one("#favorite", Button).label = (
            "♥" if track and self.store.is_favorite(track["id"]) else "♡"
        )
        self.query_one("#play-mode", Button).label = MODE_LABELS[state["mode"]] + " ▾"
        self.query_one("#volume-label", Button).label = f"{state['volume']}%"
        self.query_one("#audio-quality", Static).update(state.get("quality", ""))
        self.last_error = state["error"]
        self.render_now_status(state)
        if state["queue"] != self.last_queue:
            self.last_queue = state["queue"]
            self.refresh_library()
        if state["lyrics"] != self.last_lyric_text:
            self.last_lyric_text = state["lyrics"]
            self.lyric_lines = parse_lrc(state["lyrics"])
            self.last_lyric_index = None
        index = current_line(self.lyric_lines, state["position"], state["lyric_offset"])
        lyric_height = self.query_one("#lyrics").content_size.height
        lyric_view = (
            index,
            state["phase"],
            state.get("lyrics_status", "unavailable"),
            lyric_height,
        )
        if lyric_view != getattr(self, "last_lyric_view", None) or self.last_lyric_index is None:
            self.last_lyric_view = lyric_view
            self.last_lyric_index = index
            if not self.lyric_lines:
                if state.get("lyrics_status") == "loading":
                    text = "正在加载歌词…"
                else:
                    text = ""  # Recovery text and retry live together above the lyrics.
                self.query_one("#lyrics", Static).update(
                    Text(text, style="#789081", justify="center")
                )
            else:
                text = Text(justify="center")
                spacing = 2 if lyric_height >= 8 else 1
                count = min(7, max(1, lyric_height // spacing))
                start = max(0, index - count // 2)
                end = min(len(self.lyric_lines), start + count)
                start = max(0, end - count)
                for i in range(start, end):
                    text.append(
                        self.lyric_lines[i].text + ("\n\n" if spacing == 2 else "\n"),
                        style="bold #d7ebae" if i == index else "#789081",
                    )
                self.query_one("#lyrics", Static).update(text)

    def action_refresh_lyrics(self):
        if not self.state or not self.state.get("track"):
            return
        if self.state.get("lyrics_status") == "loading":
            return
        self.control("lyrics_retry")

    @work(group="control")
    async def control(self, operation, **arguments):
        try:
            self.state = await self.client.request(operation, **arguments)
            self.render_state(self.state)
            if operation in ("like", "queue_add", "queue_remove"):
                self.refresh_library()
        except MusicError as error:
            message = str(error)
            if "未知操作" in message:
                message = "后台仍是旧版本。退出界面后运行 wocli daemon stop，再重新打开。"
            self.notify(message, severity="error", markup=False)
