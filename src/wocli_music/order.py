import random

from .models import MusicError


class ShuffleOrder:
    """Shuffle without repeats within a round; back follows actual play history."""

    def __init__(self):
        self.remaining = []
        self.history = []
        self.cursor = -1

    def reset(self, size: int, current: int):
        self.remaining = [index for index in range(size) if index != current]
        random.shuffle(self.remaining)
        self.history = [current] if current >= 0 else []
        self.cursor = len(self.history) - 1

    def next(self, size: int, current: int) -> int:
        if self.cursor + 1 < len(self.history):
            self.cursor += 1
            return self.history[self.cursor]
        if not self.remaining:
            self.remaining = list(range(size))
            random.shuffle(self.remaining)
            if size > 1 and self.remaining[-1] == current:
                self.remaining[0], self.remaining[-1] = self.remaining[-1], self.remaining[0]
        result = self.remaining.pop() if self.remaining else current
        self.history.append(result)
        self.cursor += 1
        return result

    def previous(self) -> int:
        if self.cursor <= 0:
            raise MusicError("随机播放历史中没有上一首")
        self.cursor -= 1
        return self.history[self.cursor]
