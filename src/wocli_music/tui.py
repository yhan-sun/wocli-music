import asyncio
import random

from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import (
    Button,
    DataTable,
    Footer,
    Input,
    OptionList,
    ProgressBar,
    Select,
    Static,
)
from textual.widgets.option_list import Option

from .client import Client
from .config import Paths, Settings
from .dialogs import MODE_LABELS, ConfirmDelete, FolderPicker, LibraryPicker, ModePicker, NameDialog
from .lyrics import current_line, parse_lrc
from .models import MusicError, Track, clock
from .provider import KuwoProvider
from .store import Store


class MusicApp(App):
    """A keyboard-first music library; playback survives closing the interface."""

    TITLE = "wocli · 你的音乐库"
    CSS = """
    * { scrollbar-background: #18251c; scrollbar-color: #405937;
        scrollbar-color-hover: #789766; scrollbar-color-active: #bddb92; }
    Screen { background: #121b16; color: #dce3d2; }
    #masthead { height: 3; padding: 0 2; align-vertical: middle; }
    #wordmark { width: 1fr; color: #c2dc9c; text-style: bold; padding-top: 1; }
    #connection { width: 23; color: #829b87; text-align: right; padding-top: 1; }
    Button { min-width: 7; height: 3; border: none; background: #26382a; color: #dce3d2; }
    Button:hover { background: #3b5233; }
    Button:focus { text-style: bold; background: #405b35; }
    #library-open { display: none; min-width: 6; margin-right: 1; }
    #search-open { width: 13; margin-right: 1; }
    #quality { width: 14; height: 1; margin-top: 1; }
    Select { border: none; }
    SelectCurrent { background: #1c2b20; border: none; padding: 0 1; }
    Input { border: tall #425a3b; background: #1b2a1e; }
    Input:focus { border: tall #bddb92; }
    #body { height: 1fr; padding: 0 2; }
    #sidebar { width: 23; padding: 1 2 0 0; }
    .eyebrow { height: 2; color: #81967a; }
    #library { height: 1fr; padding: 0; border: none; background: #121b16; }
    #library > .option-list--option { padding: 0 1; }
    #library > .option-list--option-highlighted { background: #30452c; color: #d8ebba; }
    #library > .option-list--option-disabled { color: #72836b; }
    #new-folder { height: 3; width: 1fr; background: #1c2b20; }
    #folder-management { height: 3; display: none; }
    #folder-management Button { width: 1fr; min-width: 6; height: 3; background: #1c2b20; }
    #collection { width: 1fr; padding: 1 2 0 0; min-width: 24; }
    #view-heading { height: 2; }
    #view-title { width: 1fr; color: #d9e9c4; text-style: bold; }
    #view-count { width: auto; color: #829b87; }
    #list-status { height: 1; color: #829b87; }
    #searchbar { height: 3; display: none; margin-top: 1; }
    #search { width: 1fr; }
    #search-button { width: 8; margin-left: 1; }
    #list-actions { height: 3; }
    #list-actions Button { min-width: 6; height: 1; margin: 1 1 0 0; padding: 0 1; }
    #play-all { background: #bddb92; color: #152014; }
    #results { height: 1fr; background: #121b16; }
    DataTable > .datatable--header { background: #1e2c20; color: #8fa482; text-style: none; }
    DataTable > .datatable--cursor { background: #2b3c27; color: #e4efcc; }
    DataTable:focus > .datatable--cursor { background: #3b542e; color: #edf5d9; }
    #empty { height: 1fr; align: center middle; padding: 1; }
    #empty-title { text-align: center; height: auto; color: #d4e2be; text-style: bold; }
    #empty-copy { text-align: center; height: auto; margin: 1 0; color: #8b9e80; }
    #empty-search { width: 18; }
    #paging { height: 3; align: center middle; }
    #paging Button { height: 1; margin: 1 1; }
    #page { width: auto; margin: 1 1; color: #8fa482; }
    #now { width: 34%; min-width: 29; background: #18251c; padding: 1 2; }
    #song-title { height: auto; max-height: 3; text-style: bold; color: #d6e9b5; margin-bottom: 1; }
    #song-meta { height: auto; max-height: 4; color: #94ab91; }
    #lyrics { height: 1fr; padding-top: 1; content-align: center middle; }
    #hint { height: 2; color: #71846a; }
    #transport { height: 6; padding: 1 2; background: #1e2e22; }
    #time-row { height: 1; }
    #elapsed, #duration { width: 6; color: #a8be96; }
    #timeline { width: 1fr; padding: 0 1; }
    #timeline Bar { width: 1fr; }
    #timeline > .bar--bar { color: #bddb92; }
    #controls { height: 3; }
    #controls Button { height: 3; margin-right: 1; }
    #play-pause { background: #bddb92; color: #152014; }
    #favorite { min-width: 5; }
    #play-mode { min-width: 13; }
    #volume-down, #volume-up { min-width: 3; }
    #volume-label { width: 5; padding-top: 1; text-align: center; }
    #playback-message { width: 1fr; padding-top: 1; margin-left: 1; color: #a8be96; }
    Footer { background: #121b16; }
    .narrow #now { display: none; }
    .narrow #collection { padding-right: 0; }
    .compact #sidebar { width: 20; }
    .compact #connection, .compact #playback-message { display: none; }
    .tiny #library-open { display: block; }
    .tiny #sidebar, .tiny #quality, .tiny #volume-down, .tiny #volume-up,
    .tiny #volume-label { display: none; }
    .tiny #body, .tiny #masthead, .tiny #transport { padding-left: 1; padding-right: 1; }
    """
    BINDINGS = [
        Binding("ctrl+f,slash", "search", "搜索"),
        Binding("space", "play_or_toggle", "播放/暂停"),
        Binding("f", "favorite", "喜欢"),
        Binding("c", "collect", "收藏到…"),
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
        self.search_status = "搜索歌名或歌手 · Esc 返回音乐库"
        self.search_empty_title = "从一首歌开始"
        self.last_queue = None
        self.last_track = None
        self.last_lyric_text = None
        self.last_lyric_index = None
        self.lyric_lines = []
        self.favorite_ids = set()
        self.navigation_signature = None
        self.table_signature = None
        self.table_width = None

    def compose(self) -> ComposeResult:
        with Horizontal(id="masthead"):
            yield Static("wocli  /  你的音乐库", id="wordmark")
            yield Button("曲库", id="library-open")
            yield Button("⌕ 搜索  /", id="search-open")
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
            yield Static("无需登录 · 本地曲库", id="connection")
        with Horizontal(id="body"):
            with Vertical(id="sidebar"):
                yield Static("音 乐 库", classes="eyebrow")
                yield OptionList(id="library")
                yield Button("＋ 新建收藏夹", id="new-folder")
                with Horizontal(id="folder-management"):
                    yield Button("重命名", id="rename-folder")
                    yield Button("删除", id="delete-folder")
            with Vertical(id="collection"):
                with Horizontal(id="view-heading"):
                    yield Static("我喜欢", id="view-title")
                    yield Static("", id="view-count")
                yield Static("把喜欢的歌留下来。", id="list-status")
                with Horizontal(id="searchbar"):
                    yield Input(placeholder="歌名 / 歌手，Enter 搜索 · Esc 返回", id="search")
                    yield Button("搜索", id="search-button")
                with Horizontal(id="list-actions"):
                    yield Button("▶ 播放全部", id="play-all")
                    yield Button("⤨ 随机", id="play-shuffled")
                    yield Button("＋ 收藏到…", id="collect")
                    yield Button("移出", id="remove-track")
                yield DataTable(id="results", cursor_type="row")
                with Vertical(id="empty"):
                    yield Static("还没有喜欢的歌", id="empty-title")
                    yield Static("搜索一首歌，按 f 标记喜欢。", id="empty-copy")
                    yield Button("⌕ 去搜索", id="empty-search")
                with Horizontal(id="paging"):
                    yield Button("上一页", id="page-prev")
                    yield Static("—", id="page")
                    yield Button("下一页", id="page-next")
            with Vertical(id="now"):
                yield Static("正 在 播 放", classes="eyebrow")
                yield Static("留一点时间给音乐。", id="song-title")
                yield Static("选择歌曲，按 Enter 播放。", id="song-meta")
                yield Static("歌词会随着音乐在这里出现。", id="lyrics")
                yield Static("l 刷新歌词 · 退出后继续播放", id="hint")
        with Vertical(id="transport"):
            with Horizontal(id="time-row"):
                yield Static("00:00", id="elapsed")
                yield ProgressBar(total=100, show_eta=False, show_percentage=False, id="timeline")
                yield Static("00:00", id="duration")
            with Horizontal(id="controls"):
                yield Button("上一首", id="previous")
                yield Button("播放", id="play-pause")
                yield Button("下一首", id="next")
                yield Button("♡", id="favorite")
                yield Button(MODE_LABELS[self.settings.values["mode"]] + " ▾", id="play-mode")
                yield Button("−", id="volume-down")
                yield Static("60%", id="volume-label")
                yield Button("+", id="volume-up")
                yield Static("选择一首歌，开始听。", id="playback-message")
        yield Footer()

    def on_mount(self):
        self.resize_tables()
        self.refresh_library()
        self.show_view(self.active_view)
        self.query_one("#results").focus()
        self.set_interval(0.5, self.poll)
        self.set_interval(2, self.refresh_library)
        self.poll()
        self.on_resize()

    def on_resize(self):
        self.screen_stack[0].set_class(self.size.width < 110, "narrow")
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
        descriptions = {
            "favorites": "f 喜欢 / 取消喜欢 · c 收进收藏夹",
            "queue": "按 Enter 从选中歌曲继续播放 · Delete 移出",
            "history": "最近听过的歌，也值得再听一次。",
            "search": "搜索歌名或歌手 · Esc 返回音乐库",
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
        self.query_one("#list-status", Static).update(
            self.search_status
            if view == "search"
            else descriptions.get(view, "你的本地收藏夹 · c 添加歌曲 · r 重命名")
        )
        self.query_one("#searchbar").display = view == "search"
        self.query_one("#paging").display = view == "search" and bool(tracks)
        self.query_one("#page", Static).update(
            f"{self.page + 1} / {max(1, (self.total + 19) // 20)}"
        )
        self.query_one("#page-prev", Button).disabled = self.page == 0
        self.query_one("#page-next", Button).disabled = (self.page + 1) * 20 >= self.total
        self.query_one("#folder-management").display = view.startswith("playlist:")
        self.query_one("#remove-track").display = view in ("favorites", "queue") or view.startswith(
            "playlist:"
        )
        for identity in ("play-all", "play-shuffled", "collect", "remove-track"):
            self.query_one(f"#{identity}", Button).disabled = not tracks
        self.fill_table("results", tracks)
        self.query_one("#results").display = bool(tracks)
        self.query_one("#empty").display = not tracks
        self.query_one("#empty-title", Static).update(
            "还没有喜欢的歌"
            if view == "favorites"
            else self.search_empty_title
            if view == "search"
            else "这里还没有歌曲"
        )
        self.query_one("#empty-copy", Static).update(
            "输入歌名或歌手，按 Enter 搜索。"
            if view == "search"
            else "搜索歌曲，按 c 选择这个收藏夹。"
            if view.startswith("playlist:")
            else "搜索一首歌，按 f 标记喜欢。"
        )
        self.query_one("#empty-search").display = view != "search"
        if persist or changed:
            self.highlight_view()
        if changed:
            self.query_one("#results", DataTable).move_cursor(row=0)
        self.call_after_refresh(self.resize_tables)

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
        self.search_status = "正在搜索…"
        self.show_view("search", persist=False)
        try:
            total, tracks = await asyncio.to_thread(self.provider.search, keyword, page, 20)
            if serial != self.search_serial:
                return
            self.total, self.search_tracks = total, tracks
            self.store.remember(tracks)
            self.search_status = Text(
                f"{keyword} · {total:,} 首" if tracks else "没有找到歌曲，试试歌手或其他关键词。"
            )
            self.search_empty_title = "没有找到匹配的歌" if not tracks else ""
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
            self.notify("先搜索或选择一首歌。")
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
            if view:
                self.show_view(view)
                self.query_one("#results").focus()

        self.push_screen(LibraryPicker(self.store.playlists()), choose)

    def action_search(self):
        if self.active_view != "search":
            self.previous_view = self.active_view
        self.show_view("search")
        self.query_one("#search").focus()

    def action_focus_list(self):
        if self.active_view == "search":
            self.show_view(self.previous_view)
        self.query_one("#results").focus()

    def action_collect(self):
        track = self.selected() or (
            Track.from_data(self.state["track"]) if self.state and self.state["track"] else None
        )
        if not track:
            self.notify("先选择一首歌，再加入收藏夹。")
            return

        def collect(result):
            if result:
                name, _ = result
                self.store.add_to_playlist(name, track)
                self.store.save("ui.last_folder", name)
                self.refresh_library()
                self.notify(f"已收进「{name}」。", markup=False)

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

    def action_mode(self):
        mode = self.state["mode"] if self.state else self.settings.values["mode"]
        self.push_screen(
            ModePicker(mode), lambda value: self.control("mode", value=value) if value else None
        )

    @on(Button.Pressed)
    def button_pressed(self, event):
        actions = {
            "library-open": self.action_library,
            "search-open": self.action_search,
            "empty-search": self.action_search,
            "search-button": self.start_search,
            "play-pause": self.action_play_or_toggle,
            "collect": self.action_collect,
            "new-folder": self.action_new_folder,
            "rename-folder": self.action_rename_folder,
            "delete-folder": self.delete_folder,
            "remove-track": self.action_remove_track,
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
        elif identity in ("volume-down", "volume-up"):
            value = self.state["volume"] if self.state else self.settings.values["volume"]
            self.control(
                "volume", value=max(0, min(100, value + (5 if identity == "volume-up" else -5)))
            )

    def toggle_favorite(self, track):
        if not track:
            self.notify("先选择一首歌。")
            return
        liked = self.store.favorite(track)
        self.refresh_library()
        if self.state:
            self.render_state(self.state)
        self.notify("已加入我喜欢。" if liked else "已取消喜欢。")

    def action_favorite(self):
        self.toggle_favorite(self.selected())

    def action_enqueue(self):
        if track := self.selected():
            self.control("queue_add", tracks=[track.data()])

    def action_play_or_toggle(self):
        if self.focused and self.focused.id == "results":
            track, current = self.selected(), (self.state or {}).get("track")
            if track and (
                not current
                or current["id"] != track.id
                or self.state["phase"] in ("stopped", "error")
            ):
                self.play_selection()
                return
        if self.state and self.state["phase"] in ("playing", "paused"):
            self.control("toggle")
        elif self.state and self.state["queue"]:
            self.control("play")
        else:
            self.play_selection()

    def action_next(self):
        self.control("next")

    def action_prev(self):
        self.control("prev")

    def action_seek_forward(self):
        self.control("seek", seconds=5)

    def action_seek_back(self):
        self.control("seek", seconds=-5)

    @work(exclusive=True, group="poll")
    async def poll(self):
        try:
            state = await self.client.request("status", autostart=False)
            if state:
                self.state = state
                self.render_state(state)
            elif self.query("#connection"):
                self.query_one("#connection", Static).update("无需登录 · 本地曲库")
                if self.state:
                    self.state = None
                    self.query_one("#playback-message", Static).update("后台服务已关闭")
                    self.query_one("#play-pause", Button).label = "播放"
        except MusicError:
            if self.query("#connection"):
                self.query_one("#connection", Static).update("连接后台服务中…")

    def render_state(self, state):
        if not self.query("#connection"):
            return
        state = dict(state)
        if state.get("track"):
            cached = self.store.lyric(state["track"]["id"])
            if cached and parse_lrc(cached):
                state["lyrics"], state["lyrics_status"] = cached, "ready"
        if getattr(self, "refreshing_lyric_id", None) is not None and self.refreshing_lyric_id == (
            state.get("track") or {}
        ).get("id"):
            state["lyrics_status"] = "loading"
        self.query_one("#connection", Static).update("无需登录 · 已连接")
        track = state["track"]
        if track != self.last_track:
            self.last_track = track
            self.query_one("#song-title", Static).update(
                Text(track["title"] if track else "留一点时间给音乐。")
            )
            self.query_one("#song-meta", Static).update(
                Text(
                    f"{track['artist']}\n{track['album']}" if track else "选择歌曲，按 Enter 播放。"
                )
            )
            self.last_lyric_index = None
            self.refresh_library()
        self.query_one("#elapsed", Static).update(clock(state["position"]))
        self.query_one("#duration", Static).update(clock(state["duration"]))
        self.query_one("#timeline", ProgressBar).update(
            total=max(1, state["duration"]), progress=state["position"]
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
        self.query_one("#volume-label", Static).update(f"{state['volume']}%")
        message = {
            "loading": "正在解析音源…",
            "playing": f"正在播放 · {state['quality']}",
            "paused": "已暂停",
            "stopped": "已停止",
            "error": state["error"].split("。", 1)[0],
        }[state["phase"]]
        self.query_one("#playback-message", Static).update(Text(message))
        if state["error"] and state["error"] != getattr(self, "last_error", ""):
            self.notify(state["error"], severity="error", timeout=8, markup=False)
        self.last_error = state["error"]
        if state["queue"] != self.last_queue:
            self.last_queue = state["queue"]
            self.refresh_library()
        if state["lyrics"] != self.last_lyric_text:
            self.last_lyric_text = state["lyrics"]
            self.lyric_lines = parse_lrc(state["lyrics"])
            self.last_lyric_index = None
        index = current_line(self.lyric_lines, state["position"], state["lyric_offset"])
        lyric_view = (index, state["phase"], state.get("lyrics_status", "unavailable"))
        if lyric_view != getattr(self, "last_lyric_view", None) or self.last_lyric_index is None:
            self.last_lyric_view = lyric_view
            self.last_lyric_index = index
            if not self.lyric_lines:
                if state["phase"] == "error":
                    text = (
                        "此版本仅返回试听或不完整音频。\n未获取到可播放的完整版。"
                        if state.get("error_code") == "incomplete_audio"
                        else "当前音源暂不可用。\n可重试或选择其他歌曲。"
                    )
                elif state["phase"] == "loading":
                    text = "正在解析音源…"
                elif state.get("lyrics_status") == "loading":
                    text = "正在加载歌词…"
                elif not track:
                    text = "歌词会随着音乐在这里出现。"
                elif state["phase"] == "stopped":
                    text = "播放已停止。"
                else:
                    text = state.get("lyrics_error") or "暂未找到匹配歌词。\n按 l 重新获取。"
                self.query_one("#lyrics", Static).update(
                    Text(text, style="#789081", justify="center")
                )
            else:
                text = Text(justify="center")
                for i in range(max(0, index - 2), min(len(self.lyric_lines), max(0, index) + 4)):
                    text.append(
                        self.lyric_lines[i].text + "\n\n",
                        style="bold #d7ebae" if i == index else "#789081",
                    )
                self.query_one("#lyrics", Static).update(text)

    @work(exclusive=True, group="refresh-lyrics")
    async def action_refresh_lyrics(self):
        if not self.state or not self.state.get("track"):
            self.notify("请先播放一首歌。")
            return
        track = Track.from_data(self.state["track"])
        self.refreshing_lyric_id = track.id
        self.render_state(self.state)
        try:
            text = await asyncio.to_thread(self.provider.lyrics, track)
            if text and parse_lrc(text):
                self.store.lyric(track.id, text)
                self.notify("歌词已更新。")
            else:
                self.notify("暂未找到匹配的时间轴歌词。", severity="warning")
        except Exception:
            self.notify("歌词获取失败，请稍后按 l 重试。", severity="warning")
        finally:
            self.refreshing_lyric_id = None
            if self.state:
                self.render_state(self.state)

    @work(group="control")
    async def control(self, operation, **arguments):
        try:
            self.state = await self.client.request(operation, **arguments)
            self.render_state(self.state)
            if operation in ("like", "queue_add", "queue_remove"):
                self.refresh_library()
        except MusicError as error:
            self.notify(str(error), severity="error", markup=False)
