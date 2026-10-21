from rich.text import Text
from textual.message import Message
from textual.widgets import DataTable, Static


class SongTable(DataTable):
    """A single click selects; double-click and Enter activate a song."""

    def _on_click(self, event):
        row = event.style.meta.get("row", -1)
        if event.button != 1:
            event.prevent_default()
            return
        if event.chain == 1 and 0 <= row < self.row_count:
            # Textual dispatches base-class handlers too; suppress its activation.
            event.prevent_default()
            self.move_cursor(row=row)
            self.focus()
            event.stop()


class SeekBar(Static, can_focus=True):
    """Clickable timeline. Drag previews locally and seeks once on release."""

    ALLOW_SELECT = False
    DEFAULT_CSS = """
    SeekBar { height: 1; width: 1fr; padding: 0 1; }
    """

    class SeekRequested(Message):
        def __init__(self, fraction, duration, track_id):
            super().__init__()
            self.fraction, self.duration, self.track_id = fraction, duration, track_id

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.position = 0
        self.duration = 0
        self.track_id = None
        self.enabled = False
        self.dragging = False
        self.preview = None
        self.gesture = None

    def set_state(self, position, duration, track_id, enabled):
        enabled = enabled and duration > 0
        if (track_id != self.track_id or not enabled) and self.dragging:
            self.release_mouse()
            self.dragging, self.preview = False, None
        self.position, self.duration, self.track_id = position, duration, track_id
        self.enabled = enabled
        self.tooltip = "点击或拖动调整进度" if self.enabled else None
        self.refresh()

    def fraction_at(self, screen_x):
        width = self.content_region.width
        return max(0, min(1, (screen_x - self.content_region.x) / max(1, width - 1)))

    def on_mouse_down(self, event):
        if event.button != 1 or not self.enabled:
            return
        self.focus()
        self.dragging = True
        self.gesture = (self.duration, self.track_id)
        self.preview = self.fraction_at(event.screen_x)
        self.capture_mouse()
        self.refresh()
        event.stop()

    def on_mouse_move(self, event):
        if self.dragging:
            self.preview = self.fraction_at(event.screen_x)
            self.refresh()
            event.stop()

    def on_mouse_up(self, event):
        if event.button == 1 and self.dragging:
            fraction = self.fraction_at(event.screen_x)
            duration, track_id = self.gesture
            self.release_mouse()
            self.dragging, self.preview = False, None
            self.post_message(self.SeekRequested(fraction, duration, track_id))
            self.refresh()
            event.stop()

    def render(self):
        width = self.content_size.width
        if width <= 0:
            return Text("")
        fraction = (
            self.preview if self.preview is not None else self.position / max(1, self.duration)
        )
        filled = round(max(0, min(1, fraction)) * max(0, width - 1))
        if not self.enabled:
            return Text("─" * width, style="#344438")
        return Text.assemble(
            ("━" * filled, "#bddb92"), ("●", "#deedbf"), ("─" * (width - filled - 1), "#344438")
        )
