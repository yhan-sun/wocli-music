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
NameDialog, FolderPicker, ModePicker, ConfirmDelete, LibraryPicker { align: center middle; background: #0c1410 80%; }
.dialog { width: 62; max-width: 94%; height: auto; max-height: 92%;
    background: #1a2720; border: round #627d58; padding: 1 2; }
.dialog-title { height: auto; text-style: bold; color: #d5e8b4; margin-bottom: 1; }
.dialog-caption { height: auto; max-height: 3; color: #9bad9b; margin-bottom: 1; }
.dialog-error { height: auto; color: #e5a58d; }
.dialog-footer { height: auto; color: #91a18d; margin-top: 1; }
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
            yield Static("把想一起听的歌，放进同一个收藏夹。", classes="dialog-caption")
            yield Input(
                value=self.initial,
                placeholder="例如：通勤、深夜、日语收藏",
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
            yield Static("↑ ↓ 选择 · Enter 确认 · 1–4 直达 · Esc 返回", classes="dialog-footer")

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
                "将移除此收藏夹和其中的收录关系，其他收藏夹及喜欢的歌曲仍会保留。",
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
            yield Static("你的音乐库", classes="dialog-title")
            yield OptionList(
                Option("♥ 我喜欢", id="favorites"),
                Option("≡ 播放队列", id="queue"),
                Option("◷ 最近播放", id="history"),
                *[
                    Option(Text(folder["name"]), id="playlist:" + folder["name"])
                    for folder in self.folders
                ],
                id="view-options",
            )
            yield Static("↑ ↓ 选择 · Enter 打开 · Esc 返回", classes="dialog-footer")

    def on_mount(self):
        self.query_one(OptionList).focus()

    @on(OptionList.OptionSelected)
    def selected(self, event):
        self.dismiss(event.option.id)

    def action_cancel(self):
        self.dismiss(None)
