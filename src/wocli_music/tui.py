import asyncio

from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.widgets import (
    Button,
    DataTable,
    Footer,
    Input,
    Label,
    ProgressBar,
    Select,
    Static,
    TabbedContent,
    TabPane,
)

from .client import Client
from .config import MODES, Paths, Settings
from .lyrics import current_line, parse_lrc
from .models import MusicError, Track, clock
from .provider import KuwoProvider
from .store import Store

MODE_LABELS = {
    "list": "顺序播放",
    "repeat": "列表循环",
    "single": "单曲循环",
    "shuffle": "随机播放",
}


class MusicApp(App):
    """A keyboard-first workbench. Closing the UI leaves playback running."""

    TITLE = "wocli · 音乐工作台"
    CSS = """
    Screen { background: #141b19; color: #e1e6d9; }
    #masthead { height: 3; padding: 1 2 0 2; }
    #wordmark { width: 1fr; color: #c0dca0; text-style: bold; }
    #connection { width: auto; color: #8eaaa0; }
    #searchbar { height: 4; padding: 0 2; }
    Input { border: tall #35473d; background: #1d2722; }
    Input:focus { border: tall #bfdc9e; }
    #search { width: 1fr; }
    Button { min-width: 7; border: none; background: #2b3930; color: #e1e6d9; }
    Button:hover { background: #40543f; }
    Button:focus { text-style: bold; }
    #search-button { margin: 1 1; height: 1; background: #bfdc9e; color: #17221a; }
    #quality { width: 17; }
    Select { border: none; }
    SelectCurrent { border: tall #35473d; background: #1d2722; padding: 0 1; height: 3; }
    #body { height: 1fr; padding: 0 2; }
    #collection { width: 3fr; min-width: 36; padding-right: 2; }
    #list-status { height: 1; color: #8eaaa0; padding: 0 1; }
    TabbedContent { height: 1fr; }
    TabPane { padding: 1 0 0 0; }
    Tabs { background: #141b19; }
    Tab { color: #8eaaa0; }
    Tab.-active { color: #bfdc9e; }
    Underline > .underline--bar { color: #bfdc9e; background: #bfdc9e; }
    DataTable { height: 1fr; background: #141b19; }
    DataTable > .datatable--header { background: #202c25; color: #8eaaa0; text-style: none; }
    DataTable > .datatable--cursor { background: #354a35; color: #e9f3db; }
    DataTable:focus > .datatable--cursor { background: #40593c; }
    #paging { height: 3; align-horizontal: center; }
    #paging Button { margin: 0 1; }
    #page { width: auto; margin: 0 2; color: #8eaaa0; }
    #playlist-actions { height: 3; }
    #playlist-name { width: 1fr; }
    #playlist-actions Button { margin: 1 0 0 1; height: 1; }
    #playlist-songs { height: 2fr; min-height: 1; }
    #playlist-list { height: 1fr; min-height: 2; }
    #now { width: 2fr; min-width: 28; background: #1a241f; padding: 1 2; }
    .eyebrow { height: 2; color: #91a38e; }
    #song-title { height: auto; max-height: 3; text-style: bold; color: #d6e9bd; margin-bottom: 1; }
    #song-meta { height: auto; max-height: 4; color: #9bb2a2; }
    #lyrics { height: 1fr; padding-top: 2; content-align: center middle; }
    #hint { color: #789081; height: 2; }
    #transport { height: 6; padding: 1 2; background: #202d25; }
    #time-row { height: 1; }
    #timeline { width: 1fr; padding: 0 2; }
    #timeline Bar { width: 1fr; }
    #timeline > .bar--bar { color: #bfdc9e; }
    #elapsed, #duration { width: 6; color: #b8cab2; }
    #controls { height: 3; }
    #controls Button { margin-right: 1; height: 3; content-align: center middle; }
    #play-pause { background: #bfdc9e; color: #152019; }
    #play-mode { min-width: 12; }
    #volume-label { width: 8; margin-top: 1; text-align: center; }
    #playback-message { width: 1fr; margin: 1 0 0 1; color: #b8cab2; }
    Footer { background: #141b19; }
    .narrow #now { display: none; }
    .narrow #collection { padding-right: 0; }
    .narrow #playback-message { display: none; }
    .tiny #quality, .tiny #volume-down, .tiny #volume-up, .tiny #volume-label { display: none; }
    """
    BINDINGS = [
        Binding("ctrl+f,slash", "search", "搜索"),
        Binding("space", "play_or_toggle", "播放/暂停"),
        Binding("a", "enqueue", "加入队列"),
        Binding("f", "favorite", "收藏"),
        Binding("l", "refresh_lyrics", "刷新歌词"),
        Binding("ctrl+right", "next", "下一首"),
        Binding("ctrl+left", "prev", "上一首"),
        Binding("right", "seek_forward", "+5s", show=False),
        Binding("left", "seek_back", "−5s", show=False),
        Binding("escape", "focus_list", "返回列表", show=False),
        Binding("q,ctrl+q,ctrl+c", "quit", "退出界面"),
    ]

    def __init__(self, *, direct=False, paths=None):
        super().__init__()
        self.paths = paths or Paths.discover()
        self.settings = Settings(self.paths)
        self.store = Store(self.paths)
        self.provider = KuwoProvider(direct=direct or self.settings.values["direct"])
        self.client = Client(self.paths)
        self.rows = {}
        self.state = None
        self.keyword = ""
        self.page = 0
        self.total = 0
        self.search_serial = 0
        self.last_queue = None
        self.last_lyric_text = None
        self.lyric_lines = []
        self.last_lyric_index = None
        self.last_track = None
        self.polling = False
        self.selected_playlist = None

    def compose(self) -> ComposeResult:
        with Horizontal(id="masthead"):
            yield Static("W O C L I   /   MUSIC", id="wordmark")
            yield Static("无需登录 · 本地曲库", id="connection")
        with Horizontal(id="searchbar"):
            yield Input(placeholder="搜索歌名 / 歌手，按 Enter 开始", id="search")
            yield Button("搜索", id="search-button")
            yield Select(
                [
                    ("MP3 · 320k", "320k"),
                    ("MP3 · 128k", "128k"),
                    ("FLAC · 无损", "flac"),
                    ("最佳可用", "best"),
                ],
                value=self.settings.values["quality"],
                allow_blank=False,
                id="quality",
            )
        with Horizontal(id="body"):
            with Vertical(id="collection"):
                yield Static("搜索一首歌，从这里开始。", id="list-status")
                with TabbedContent(initial="tab-search", id="tabs"):
                    with TabPane("搜索", id="tab-search"):
                        yield DataTable(id="results", cursor_type="row", zebra_stripes=False)
                        with Horizontal(id="paging"):
                            yield Button("上一页", id="page-prev")
                            yield Label("—", id="page")
                            yield Button("下一页", id="page-next")
                    with TabPane("队列", id="tab-queue"):
                        yield DataTable(id="queue", cursor_type="row")
                    with TabPane("收藏", id="tab-favorites"):
                        yield DataTable(id="favorites", cursor_type="row")
                    with TabPane("历史", id="tab-history"):
                        yield DataTable(id="history", cursor_type="row")
                    with TabPane("歌单", id="tab-playlists"):
                        with Horizontal(id="playlist-actions"):
                            yield Input(placeholder="歌单名称", id="playlist-name")
                            yield Button("创建", id="playlist-create")
                            yield Button("加入", id="playlist-add")
                        yield DataTable(id="playlist-list", cursor_type="row")
                        yield DataTable(id="playlist-songs", cursor_type="row")
            with Vertical(id="now"):
                yield Static("N O W  P L A Y I N G", classes="eyebrow")
                yield Static("留一点时间给音乐。", id="song-title")
                yield Static("选择歌曲，按 Enter 播放。", id="song-meta")
                yield Static("歌词会随着音乐在这里出现。", id="lyrics")
                yield Static("退出界面后，音乐继续播放。", id="hint")
        with Vertical(id="transport"):
            with Horizontal(id="time-row"):
                yield Static("00:00", id="elapsed")
                yield ProgressBar(total=100, show_eta=False, show_percentage=False, id="timeline")
                yield Static("00:00", id="duration")
            with Horizontal(id="controls"):
                yield Button("上一首", id="previous")
                yield Button("播放", id="play-pause")
                yield Button("下一首", id="next")
                yield Button("♡ 收藏", id="favorite")
                yield Button(MODE_LABELS[self.settings.values["mode"]], id="play-mode")
                yield Button("−", id="volume-down")
                yield Static("60%", id="volume-label")
                yield Button("+", id="volume-up")
                yield Static("就绪", id="playback-message")
        yield Footer()

    def on_mount(self):
        for name in ("results", "queue", "favorites", "history", "playlist-songs"):
            table = self.query_one(f"#{name}", DataTable)
            table.add_column("#", key="number", width=3)
            table.add_column("歌曲", key="title", width=24)
            table.add_column("歌手", key="artist", width=12)
            table.add_column("时长", key="duration", width=5)
            self.rows[name] = []
        self.query_one("#playlist-list", DataTable).add_columns("本地歌单", "歌曲数")
        self.refresh_library()
        saved = self.store.load("queue", {"tracks": []})
        self.fill_table("queue", [Track.from_data(t) for t in saved["tracks"]])
        self.query_one("#search", Input).focus()
        self.set_interval(0.5, self.poll)
        self.poll()
        self.on_resize()

    def on_resize(self):
        self.screen.set_class(self.size.width < 96, "narrow")
        self.screen.set_class(self.size.width < 72, "tiny")
        self.call_after_refresh(self.resize_tables)

    def resize_tables(self):
        for name, tracks in self.rows.items():
            table = self.query_one(f"#{name}", DataTable)
            cursor = table.cursor_row
            available = max(16, table.size.width - 19)
            title_width = max(10, int(available * 0.63))
            artist_width = max(6, available - title_width)
            table.clear(columns=True)
            table.add_column("#", key="number", width=3)
            table.add_column("歌曲", key="title", width=title_width)
            table.add_column("歌手", key="artist", width=artist_width)
            table.add_column("时长", key="duration", width=5)
            self.fill_table(name, tracks)
            if tracks:
                table.move_cursor(row=min(cursor, len(tracks) - 1))

    def on_unmount(self):
        self.store.close()

    def check_action(self, action, parameters):
        if isinstance(self.focused, Input) and action in (
            "play_or_toggle",
            "enqueue",
            "favorite",
            "refresh_lyrics",
            "seek_forward",
            "seek_back",
            "quit",
        ):
            # Ctrl+Q / Ctrl+C remain handled by the priority key hook below.
            return False
        return True

    def on_key(self, event):
        if event.key in ("ctrl+q", "ctrl+c"):
            event.stop()
            self.exit()

    def fill_table(self, name, tracks):
        table = self.query_one(f"#{name}", DataTable)
        cursor = table.cursor_row
        table.clear()
        self.rows[name] = tracks
        for i, track in enumerate(tracks):
            table.add_row(
                str(i + 1), Text(track.title), Text(track.artist), clock(track.duration), key=str(i)
            )
        if tracks:
            table.move_cursor(row=min(cursor, len(tracks) - 1))

    def refresh_library(self):
        self.fill_table("favorites", self.store.favorites())
        self.fill_table("history", self.store.history())
        table = self.query_one("#playlist-list", DataTable)
        table.clear()
        for playlist in self.store.playlists():
            table.add_row(Text(playlist["name"]), str(playlist["count"]), key=playlist["name"])

    def selected(self) -> Track | None:
        focused = self.focused
        if isinstance(focused, DataTable) and focused.id in self.rows:
            tracks = self.rows[focused.id]
            return tracks[focused.cursor_row] if focused.cursor_row < len(tracks) else None
        active = self.query_one("#tabs", TabbedContent).active
        name = {
            "tab-search": "results",
            "tab-queue": "queue",
            "tab-favorites": "favorites",
            "tab-history": "history",
            "tab-playlists": "playlist-songs",
        }.get(active)
        if name:
            tracks = self.rows[name]
            index = self.query_one(f"#{name}", DataTable).cursor_row
            return tracks[index] if index < len(tracks) else None
        return None

    @on(Input.Submitted, "#search")
    def search_submitted(self, event):
        self.start_search()

    @on(Select.Changed, "#quality")
    def quality_changed(self, event):
        if event.value != Select.BLANK and event.value != self.settings.values["quality"]:
            self.save_quality(str(event.value))

    @work(exclusive=True, group="quality")
    async def save_quality(self, value):
        try:
            state = await self.client.request("status", autostart=False)
            if not self.query("#connection"):
                return
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
            self.fetch_results()

    @work(exclusive=True, group="search")
    async def fetch_results(self):
        self.search_serial += 1
        serial = self.search_serial
        self.query_one("#tabs", TabbedContent).active = "tab-search"
        self.query_one("#list-status", Static).update("正在搜索…")
        try:
            total, tracks = await asyncio.to_thread(
                self.provider.search, self.keyword, self.page, 20
            )
            if serial != self.search_serial:
                return
            self.total = total
            self.store.remember(tracks)
            self.fill_table("results", tracks)
            self.query_one("#list-status", Static).update(
                Text(
                    f"{self.keyword}  ·  {total:,} 首"
                    if tracks
                    else "没有找到歌曲，试试歌手或其他关键词。"
                )
            )
            self.query_one("#page", Label).update(f"{self.page + 1} / {max(1, (total + 19) // 20)}")
            self.query_one("#page-prev", Button).disabled = self.page == 0
            self.query_one("#page-next", Button).disabled = (self.page + 1) * 20 >= total
            self.query_one("#results", DataTable).focus()
        except MusicError as error:
            self.query_one("#list-status", Static).update(Text(str(error), style="#e2a194"))

    @on(DataTable.RowSelected)
    def row_selected(self, event):
        if event.data_table.id == "playlist-list":
            self.selected_playlist = str(event.row_key.value)
            self.query_one("#playlist-name", Input).value = self.selected_playlist
            self.fill_table("playlist-songs", self.store.playlist(self.selected_playlist))
            self.query_one("#playlist-songs", DataTable).focus()
        elif event.data_table.id in self.rows:
            self.play_selection()

    @work(exclusive=True, group="play")
    async def play_selection(self):
        track = self.selected()
        if not track:
            self.notify("先搜索或选择一首歌。")
            return
        try:
            if isinstance(self.focused, DataTable) and self.focused.id == "queue":
                await self.client.request(
                    "play",
                    index=self.focused.cursor_row,
                    quality=self.query_one("#quality", Select).value,
                )
            elif isinstance(self.focused, DataTable) and self.focused.id in (
                "playlist-songs",
                "favorites",
            ):
                await self.client.request(
                    "play",
                    tracks=[t.data() for t in self.rows[self.focused.id]],
                    index=self.focused.cursor_row,
                    quality=self.query_one("#quality", Select).value,
                )
            else:
                await self.client.request(
                    "play",
                    tracks=[track.data()],
                    index=0,
                    quality=self.query_one("#quality", Select).value,
                )
            self.last_track = None
            self.poll()
        except MusicError as error:
            self.notify(str(error), severity="error", markup=False)

    @work(exclusive=True, group="poll")
    async def poll(self):
        try:
            state = await self.client.request("status", autostart=False)
            if state:
                self.state = state
                self.render_state(state)
            else:
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
        self.query_one("#connection", Static).update("无需登录 · 播放服务已连接")
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
        self.query_one("#favorite", Button).label = "♥ 已收藏" if state["liked"] else "♡ 收藏"
        self.query_one("#play-mode", Button).label = MODE_LABELS[state["mode"]]
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
            self.fill_table("queue", [Track.from_data(t) for t in state["queue"]])
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
            if operation == "like":
                self.refresh_library()
        except MusicError as error:
            self.notify(str(error), severity="error", markup=False)

    @on(Button.Pressed)
    def button_pressed(self, event):
        identity = event.button.id
        if identity == "search-button":
            self.start_search()
        elif identity in ("page-prev", "page-next"):
            self.page += -1 if identity == "page-prev" else 1
            self.fetch_results()
        elif identity == "play-pause":
            self.action_play_or_toggle()
        elif identity in ("previous", "next"):
            self.control("prev" if identity == "previous" else "next")
        elif identity == "favorite":
            self.action_favorite()
        elif identity == "play-mode":
            mode = self.state["mode"] if self.state else self.settings.values["mode"]
            self.control("mode", value=MODES[(MODES.index(mode) + 1) % len(MODES)])
        elif identity in ("volume-down", "volume-up"):
            value = self.state["volume"] if self.state else self.settings.values["volume"]
            self.control(
                "volume", value=max(0, min(100, value + (5 if identity == "volume-up" else -5)))
            )
        elif identity in ("playlist-create", "playlist-add"):
            name = self.query_one("#playlist-name", Input).value.strip()
            try:
                if identity == "playlist-create":
                    self.store.create_playlist(name)
                else:
                    track = self.selected()
                    if not track and self.state and self.state["track"]:
                        track = Track.from_data(self.state["track"])
                    if not track:
                        raise MusicError("先播放一首歌或选择歌单中的歌曲，再加入歌单。")
                    self.store.add_to_playlist(name, track)
                    self.fill_table("playlist-songs", self.store.playlist(name))
                self.refresh_library()
                self.notify("已更新本地歌单。")
            except MusicError as error:
                self.notify(str(error), severity="warning", markup=False)

    def action_search(self):
        self.query_one("#search", Input).focus()

    def action_focus_list(self):
        active = self.query_one("#tabs", TabbedContent).active
        name = {
            "tab-search": "results",
            "tab-queue": "queue",
            "tab-favorites": "favorites",
            "tab-history": "history",
            "tab-playlists": "playlist-list",
        }[active]
        self.query_one(f"#{name}", DataTable).focus()

    def action_play_or_toggle(self):
        if isinstance(self.focused, DataTable) and self.focused.id in self.rows:
            track = self.selected()
            current = self.state["track"] if self.state else None
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

    def action_enqueue(self):
        if track := self.selected():
            self.control("queue_add", tracks=[track.data()])
            self.notify("已加入播放队列。")

    def action_favorite(self):
        track = self.selected() if isinstance(self.focused, DataTable) else None
        self.control("like", **({"track": track.data()} if track else {}))

    def action_next(self):
        self.control("next")

    def action_prev(self):
        self.control("prev")

    def action_seek_forward(self):
        self.control("seek", seconds=5)

    def action_seek_back(self):
        self.control("seek", seconds=-5)
