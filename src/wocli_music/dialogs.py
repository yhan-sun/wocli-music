from pathlib import Path

from rich.text import Text
from textual import on
from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import Button, Input, OptionList, Select, Static
from textual.widgets.option_list import Option

from .config import MODES

MODE_LABELS = {
    "list": "顺序播放",
    "repeat": "列表循环",
    "single": "单曲循环",
    "shuffle": "随机播放",
}
MODE_DESCRIPTIONS = {
    "list": "依次播放，最后一首结束后停止",
    "repeat": "播完队列，再从第一首开始",
    "single": "重复当前歌曲，切歌键仍可切换",
    "shuffle": "每轮不重复，上一首返回实际播放历史",
}

DIALOG_CSS = """
NameDialog, FolderPicker, ModePicker, ConfirmDelete, LibraryPicker, TrackActions, DownloadDialog, DownloadList, QualityPicker, FolderActions, VolumeDialog, ShortcutHelp { align: center middle; background: #0c1410 80%; }
.dialog { width: 62; max-width: 94%; height: auto; max-height: 92%;
    background: #1a2720; border: round #627d58; padding: 1 2; }
.dialog-title { height: auto; text-style: bold; color: #d5e8b4; margin-bottom: 1; }
.dialog-caption { height: auto; max-height: 5; color: #9bad9b; margin-bottom: 1; }
.dialog-error { height: auto; color: #e5a58d; }
.dialog Input { background: #142019; border: tall #526b49; }
.dialog SelectCurrent { background: #142019; border: tall #526b49; }
.dialog OptionList { height: auto; max-height: 18; border: none; background: #1a2720; }
.dialog OptionList > .option-list--option-highlighted { background: #364c31; color: #e2efcc; }
.dialog-actions { height: 3; margin-top: 1; align-horizontal: right; }
.dialog Button { border: none; min-width: 10; margin-left: 1; background: #30402c; }
.dialog .primary { background: #bad798; color: #142013; }
.dialog .danger { background: #b96650; color: #faf0de; }
#folder-new { display: none; }
#folder-new.visible { display: block; }
DownloadDialog .dialog-caption { margin-bottom: 0; max-height: 2; }
DownloadDialog .dialog-actions { height: 1; margin-top: 1; }
DownloadDialog Select { height: 1; margin-bottom: 1; }
#download-quality SelectCurrent { height: 1; border: none; }
DownloadDialog Checkbox { height: 1; margin-bottom: 1; }
ShortcutHelp .dialog-actions { height: 1; margin-top: 1; }
"""


class NameDialog(ModalScreen[str | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "取消")]

    def __init__(self, title="新建收藏夹", initial="", existing=()):
        super().__init__()
        self.title_text, self.initial, self.existing = title, initial, set(existing)

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Static(self.title_text, classes="dialog-title")
            yield Input(
                value=self.initial,
                placeholder="收藏夹名称",
                id="folder-name",
            )
            yield Static("", id="dialog-error", classes="dialog-error")
            with Horizontal(classes="dialog-actions"):
                yield Button("取消", id="cancel")
                yield Button("保存", id="save", classes="primary")

    def on_mount(self):
        self.query_one(Input).focus()

    @on(Input.Submitted)
    def submitted(self):
        self.commit()

    @on(Button.Pressed)
    def pressed(self, event):
        self.commit() if event.button.id == "save" else self.dismiss(None)

    def commit(self):
        name = self.query_one(Input).value.strip()
        error = (
            "请输入 1–100 字的名称"
            if not name or len(name) > 100
            else "已存在同名收藏夹"
            if name != self.initial and name in self.existing
            else ""
        )
        if error:
            self.query_one("#dialog-error", Static).update(error)
        else:
            self.dismiss(name)

    def action_cancel(self):
        self.dismiss(None)


class FolderPicker(ModalScreen[tuple[str, bool] | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "取消")]

    def __init__(self, track, folders, memberships=(), preferred=None):
        super().__init__()
        self.track = track
        self.folders = folders
        self.names = [folder["name"] for folder in folders]
        self.memberships = set(memberships)
        self.initial = (
            preferred if preferred in self.names else self.names[0] if self.names else "new"
        )

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Static("加入收藏夹", classes="dialog-title")
            yield Static(Text(f"{self.track.title}\n{self.track.artist}"), classes="dialog-caption")
            options = [
                (
                    Text(
                        f"{folder['name']} · {folder['count']} 首"
                        + (" · 已收录" if folder["name"] in self.memberships else "")
                    ),
                    "folder:" + folder["name"],
                )
                for folder in self.folders
            ]
            yield Select(
                [*options, ("＋ 新建收藏夹…", "new")],
                value="folder:" + self.initial if self.initial in self.names else "new",
                allow_blank=False,
                id="folder-select",
            )
            yield Input(placeholder="新收藏夹名称", id="folder-new")
            yield Static("", id="dialog-error", classes="dialog-error")
            with Horizontal(classes="dialog-actions"):
                yield Button("取消", id="cancel")
                yield Button("加入", id="save", classes="primary")

    def on_mount(self):
        self.toggle_name()

    @on(Select.Changed, "#folder-select")
    def changed(self):
        self.toggle_name()

    def toggle_name(self):
        new = self.query_one("#folder-select", Select).value == "new"
        self.query_one("#folder-new", Input).set_class(new, "visible")
        self.query_one("#folder-new" if new else "#folder-select").focus()

    @on(Input.Submitted)
    def submitted(self):
        self.commit()

    @on(Button.Pressed)
    def pressed(self, event):
        self.commit() if event.button.id == "save" else self.dismiss(None)

    def commit(self):
        value = self.query_one("#folder-select", Select).value
        is_new = value == "new"
        name = (
            self.query_one("#folder-new", Input).value.strip()
            if is_new
            else str(value).removeprefix("folder:")
        )
        error = (
            "请输入 1–100 字的名称"
            if not name or len(name) > 100
            else "该收藏夹已存在，请从列表中选择"
            if is_new and name in self.names
            else ""
        )
        if error:
            self.query_one("#dialog-error", Static).update(error)
        else:
            self.dismiss((name, is_new))

    def action_cancel(self):
        self.dismiss(None)


class ModePicker(ModalScreen[str | None]):
    CSS = DIALOG_CSS
    BINDINGS = [
        ("escape,q", "cancel", "返回"),
        ("1", "choose('list')", "顺序"),
        ("2", "choose('repeat')", "循环"),
        ("3", "choose('single')", "单曲"),
        ("4", "choose('shuffle')", "随机"),
    ]

    def __init__(self, current):
        super().__init__()
        self.current = current

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Static("播放方式", classes="dialog-title")
            options = []
            for i, mode in enumerate(MODES, 1):
                text = Text(
                    f"{'●' if mode == self.current else '○'}  {i}  {MODE_LABELS[mode]}\n",
                    style="bold #d5e8b4" if mode == self.current else "#d8dfcf",
                )
                text.append(f"       {MODE_DESCRIPTIONS[mode]}\n", style="#9bad9b")
                options.append(Option(text, id=mode))
            yield OptionList(*options, id="mode-options")

    def on_mount(self):
        options = self.query_one(OptionList)
        options.highlighted = MODES.index(self.current)
        options.focus()

    @on(OptionList.OptionSelected)
    def selected(self, event):
        self.dismiss(event.option.id)

    def action_choose(self, value):
        self.dismiss(value)

    def action_cancel(self):
        self.dismiss(None)


class ConfirmDelete(ModalScreen[bool]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "取消")]

    def __init__(self, name):
        super().__init__()
        self.folder_name = name

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Static(Text(f"删除收藏夹「{self.folder_name}」？"), classes="dialog-title")
            yield Static(
                "仅删除此收藏夹，保留歌曲与其他收藏。",
                classes="dialog-caption",
            )
            with Horizontal(classes="dialog-actions"):
                yield Button("取消", id="cancel")
                yield Button("删除收藏夹", id="delete-confirm", classes="danger")

    @on(Button.Pressed)
    def pressed(self, event):
        self.dismiss(event.button.id == "delete-confirm")

    def action_cancel(self):
        self.dismiss(False)


class LibraryPicker(ModalScreen[str | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "返回")]

    def __init__(self, folders):
        super().__init__()
        self.folders = folders

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static("曲库", classes="dialog-title")
            yield OptionList(
                Option("♥ 我喜欢", id="favorites"),
                Option("≡ 播放队列", id="queue"),
                Option("◷ 最近播放", id="history"),
                *[
                    Option(Text(folder["name"]), id="playlist:" + folder["name"])
                    for folder in self.folders
                ],
                Option("＋ 新建收藏夹…", id="create-folder"),
                id="view-options",
            )

    def on_mount(self):
        self.query_one(OptionList).focus()

    @on(OptionList.OptionSelected)
    def selected(self, event):
        self.dismiss(event.option.id)

    def action_cancel(self):
        self.dismiss(None)


class TrackActions(ModalScreen[str | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "返回")]

    def __init__(self, track, *, liked=False, removable=False, current=False):
        super().__init__()
        self.track, self.liked, self.removable = track, liked, removable
        self.current = current

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static(Text(self.track.title), classes="dialog-title")
            yield Static(Text(self.track.artist), classes="dialog-caption")
            options = [
                Option("重新播放" if self.current else "播放", id="play"),
                Option("加入队列", id="enqueue"),
                Option("取消喜欢" if self.liked else "喜欢", id="like"),
                Option("收藏到…", id="collect"),
            ]
            if self.current:
                options.append(Option("刷新歌词", id="lyrics"))
            if not self.track.path:
                options.append(Option("下载…", id="download"))
            if self.removable:
                options.append(Option("移出列表", id="remove"))
            yield OptionList(*options, id="track-options")

    def on_mount(self):
        self.query_one(OptionList).focus()

    @on(OptionList.OptionSelected)
    def selected(self, event):
        self.dismiss(event.option.id)

    def action_cancel(self):
        self.dismiss(None)


class DownloadDialog(ModalScreen[dict | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "取消")]

    def __init__(self, track, directory, quality):
        super().__init__()
        self.track, self.directory, self.quality = track, directory, quality

    def compose(self):
        from textual.widgets import Checkbox

        with Vertical(classes="dialog"):
            yield Static("下载歌曲", classes="dialog-title")
            yield Static(Text(f"{self.track.title}\n{self.track.artist}"), classes="dialog-caption")
            yield Static("保存到目录", classes="dialog-caption")
            yield Input(
                value=self.directory, placeholder="下载目录的完整路径", id="download-directory"
            )
            yield Select(
                [
                    ("最佳可用", "best"),
                    ("FLAC 无损", "flac"),
                    ("MP3 320k", "320k"),
                    ("MP3 128k", "128k"),
                ],
                value=self.quality,
                allow_blank=False,
                compact=True,
                id="download-quality",
            )
            yield Checkbox("同时保存 LRC 歌词", value=True, compact=True, id="download-lyrics")
            yield Static("", id="dialog-error", classes="dialog-error")
            with Horizontal(classes="dialog-actions"):
                yield Button("取消", id="cancel")
                yield Button("开始下载", id="download-start", classes="primary")

    @on(Button.Pressed)
    def pressed(self, event):
        from textual.widgets import Checkbox

        if event.button.id != "download-start":
            self.dismiss(None)
            return
        directory = self.query_one("#download-directory", Input).value.strip()
        if not directory:
            self.query_one("#dialog-error", Static).update("请输入保存目录")
            return
        self.dismiss(
            dict(
                directory=str(Path(directory).expanduser().resolve()),
                quality=self.query_one("#download-quality", Select).value,
                lyrics=self.query_one("#download-lyrics", Checkbox).value,
            )
        )

    def action_cancel(self):
        self.dismiss(None)


DOWNLOAD_PHASES = {
    "queued": "等待下载",
    "resolving": "解析音源",
    "downloading": "下载中",
    "lyrics": "保存歌词",
    "done": "已完成",
    "error": "失败",
    "cancelled": "已取消",
}


class DownloadList(ModalScreen[tuple | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "返回")]

    def __init__(self, get_jobs):
        super().__init__()
        self.get_jobs = get_jobs
        self.jobs = []
        self.signature = None

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static("下载任务", classes="dialog-title")
            yield OptionList(id="download-options")
            yield Static(
                "0 项",
                id="download-detail",
                classes="dialog-caption",
            )
            with Horizontal(classes="dialog-actions"):
                yield Button("返回", id="cancel")
                yield Button("取消任务", id="cancel-job")
                yield Button("重试下载", id="retry-job", classes="primary")

    def on_mount(self):
        self.refresh_jobs()
        self.set_interval(0.5, self.refresh_jobs)
        self.query_one(OptionList).focus()

    def refresh_jobs(self):
        self.jobs = list(reversed(self.get_jobs()))
        signature = [
            (job["id"], job["phase"], job["received"], job.get("error")) for job in self.jobs
        ]
        options = self.query_one(OptionList)
        if signature != self.signature:
            cursor = options.highlighted or 0
            previous = options.get_option_at_index(cursor).id if options.option_count else None
            options.clear_options().add_options(
                [
                    Option(
                        Text(f"{DOWNLOAD_PHASES[job['phase']]} · {job['track']['title']}"),
                        id=job["id"],
                    )
                    for job in self.jobs
                ]
            )
            if self.jobs:
                options.highlighted = next(
                    (i for i, job in enumerate(self.jobs) if job["id"] == previous),
                    min(cursor, len(self.jobs) - 1),
                )
            self.signature = signature
        self.details()

    @on(OptionList.OptionHighlighted)
    def details(self):
        cursor = self.query_one(OptionList).highlighted
        job = self.jobs[cursor] if cursor is not None and cursor < len(self.jobs) else None
        if job:
            size = f"{job['received'] / 1048576:.1f} / {job['total'] / 1048576:.1f} MB"
            content = job["error"] or job.get("warning") or size
            if job["path"]:
                content += "\n" + job["path"]
            self.query_one("#download-detail", Static).update(Text(content))
        self.query_one("#cancel-job", Button).disabled = not job or job["phase"] not in (
            "queued",
            "resolving",
            "downloading",
            "lyrics",
        )
        self.query_one("#retry-job", Button).disabled = not job or job["phase"] not in (
            "error",
            "cancelled",
        )

    @on(Button.Pressed)
    def pressed(self, event):
        cursor = self.query_one(OptionList).highlighted
        if event.button.id == "cancel" or cursor is None:
            self.dismiss(None)
        else:
            self.dismiss(
                (
                    "download_cancel" if event.button.id == "cancel-job" else "download_retry",
                    self.jobs[cursor]["id"],
                )
            )

    def action_cancel(self):
        self.dismiss(None)


class QualityPicker(ModalScreen[str | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "返回")]

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static("选择音质并重试", classes="dialog-title")
            yield OptionList(
                Option("最佳可用", id="best"),
                Option("FLAC 无损", id="flac"),
                Option("MP3 320k", id="320k"),
                Option("MP3 128k", id="128k"),
                id="quality-options",
            )

    def on_mount(self):
        self.query_one(OptionList).focus()

    @on(OptionList.OptionSelected)
    def selected(self, event):
        self.dismiss(event.option.id)

    def action_cancel(self):
        self.dismiss(None)


class FolderActions(ModalScreen[str | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "返回")]

    def __init__(self, folder):
        super().__init__()
        self.folder = folder

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static(Text(self.folder), classes="dialog-title")
            yield OptionList(
                Option("重命名收藏夹", id="rename"),
                Option("删除收藏夹…", id="delete"),
                id="folder-actions",
            )

    def on_mount(self):
        self.query_one(OptionList).focus()

    @on(OptionList.OptionSelected)
    def selected(self, event):
        self.dismiss(event.option.id)

    def action_cancel(self):
        self.dismiss(None)


class VolumeDialog(ModalScreen[int | None]):
    CSS = DIALOG_CSS
    BINDINGS = [("escape", "cancel", "返回")]

    def __init__(self, value):
        super().__init__()
        self.value = value

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static("播放音量", classes="dialog-title")
            yield Input(
                value=str(self.value), placeholder="0–100", type="integer", id="volume-input"
            )
            yield Static("", id="dialog-error", classes="dialog-error")
            with Horizontal(classes="dialog-actions"):
                yield Button("取消", id="cancel")
                yield Button("应用", id="apply-volume", classes="primary")

    def on_mount(self):
        self.query_one(Input).focus()

    @on(Input.Submitted)
    def submit(self):
        self.commit()

    @on(Button.Pressed)
    def pressed(self, event):
        self.commit() if event.button.id == "apply-volume" else self.dismiss(None)

    def commit(self):
        try:
            value = int(self.query_one(Input).value)
            if not 0 <= value <= 100:
                raise ValueError
        except ValueError:
            self.query_one("#dialog-error", Static).update("音量应为 0–100 的整数")
            return
        self.dismiss(value)

    def action_cancel(self):
        self.dismiss(None)


class ShortcutHelp(ModalScreen):
    CSS = DIALOG_CSS
    BINDINGS = [("escape,question_mark", "cancel", "返回")]

    def compose(self):
        with Vertical(classes="dialog"):
            yield Static("键盘操作", classes="dialog-title")
            yield Static(
                "Enter / 双击   播放选中的歌曲\n空格           暂停 / 继续；加载中取消\n/              搜索，Esc 返回\nx              所选歌曲操作\nf / c          喜欢 / 收藏到…\nd              下载歌曲\nv / l          展开歌词 / 刷新歌词\nm              播放方式\n+ / -          调整音量\nCtrl+← / →     上一首 / 下一首\nCtrl+R         重试当前音源\nb / n / r      曲库 / 新建夹 / 重命名夹\nDelete         移出当前列表\nq / Ctrl+Q     退出界面，音乐继续播放",
                id="shortcut-list",
            )
            with Horizontal(classes="dialog-actions"):
                yield Button("返回", id="help-close", classes="primary")

    @on(Button.Pressed)
    def pressed(self):
        self.dismiss()

    def action_cancel(self):
        self.dismiss()
