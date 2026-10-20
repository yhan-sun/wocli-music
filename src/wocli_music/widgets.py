from textual.widgets import DataTable


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
