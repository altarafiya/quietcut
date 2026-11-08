"""The editor's single video track, time ruler and mouse editing controls."""
from bisect import bisect_right
from math import floor
from pathlib import Path

from PyQt6.QtCore import QRectF, Qt, pyqtSignal
from PyQt6.QtGui import QColor, QPainter, QPen
from PyQt6.QtWidgets import QWidget

from interface import glyph, timecode


class Timeline(QWidget):
    seeked = pyqtSignal(float)
    selected = pyqtSignal(int)
    trimmed = pyqtSignal(int, bool, float, bool)
    reordered = pyqtSignal(int, int)
    view_changed = pyqtSignal()
    ruler_height = 29
    intervals = (.1, .5, 1, 2, 5, 10, 30, 60, 120, 300, 600, 1800, 3600)

    def __init__(self, colors):
        super().__init__()
        self.colors = colors
        self.setFixedHeight(132)
        self.setAccessibleName("Video timeline and time ruler")
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setMouseTracking(True)
        self.clips = []
        self.offsets = [0.]
        self.playhead = 0.
        self.selection = -1
        self.cuts = []
        self.pps = 1.
        self.offset = 0.
        self.fitted = True
        self.drag = None

    @property
    def duration(self):
        return self.offsets[-1]

    @property
    def max_offset(self):
        return max(0., self.duration - self.width() / self.pps)

    def set_clips(self, clips):
        self.clips = clips
        self.offsets = [0.]
        for clip in clips:
            self.offsets.append(self.offsets[-1] + clip.length)
        if self.fitted:
            self.fit()
        else:
            self.scroll(self.offset)
        self.update()

    def x_at(self, timestamp):
        return (timestamp - self.offset) * self.pps

    def time_at(self, x):
        return max(0., min(self.duration, self.offset + x / self.pps))

    def index_at(self, timestamp):
        return min(len(self.clips) - 1, bisect_right(self.offsets, timestamp) - 1)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self.fitted:
            self.fit()
        else:
            self.scroll(self.offset)

    def fit(self):
        self.fitted = True
        self.pps = max(.0001, (self.width() - 2) / (self.duration or 1))
        self.scroll(0.)

    def scroll(self, offset):
        self.offset = max(0., min(offset, self.max_offset))
        self.view_changed.emit()
        self.update()

    def zoom(self, factor, anchor=None):
        if not self.clips:
            return
        if anchor is None:
            anchor = max(0., min(self.width(), self.x_at(self.playhead)))
        timestamp = self.time_at(anchor)
        self.pps = max(.0001, min(1500., self.pps * factor))
        self.fitted = False
        self.scroll(timestamp - anchor / self.pps)

    def ensure_visible(self):
        if self.x_at(self.playhead) >= self.width() or self.x_at(self.playhead) < 0:
            self.scroll(self.playhead - self.width() * .12 / self.pps)

    def paintEvent(self, event):
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        tone = self.colors()
        frame = QRectF(self.rect()).adjusted(.5, .5, -.5, -.5)
        painter.setPen(QPen(QColor(tone.line)))
        painter.setBrush(QColor(tone.panel))
        painter.drawRoundedRect(frame, 8, 8)
        if not self.clips:
            return
        painter.setClipRect(frame.adjusted(1, 0, -1, -1))
        step = next((step for step in self.intervals if step * self.pps >= 70), self.intervals[-1])
        while step * self.pps < 50:
            step *= 2
        tick = floor(self.offset / step) * step
        end = self.offset + self.width() / self.pps
        painter.setRenderHint(QPainter.RenderHint.Antialiasing, False)
        while tick < end:
            x = round(self.x_at(tick))
            painter.setPen(QColor(tone.muted))
            painter.drawText(x + 5, 18, timecode(tick))
            painter.setPen(QColor(tone.line))
            painter.drawLine(x, 25, x, 131)
            tick += step
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        for i, clip in enumerate(self.clips):
            left, right = self.x_at(self.offsets[i]), self.x_at(self.offsets[i + 1])
            if right < 0 or left > self.width():
                continue
            selected = i == self.selection
            painter.setPen(QPen(QColor(tone.accent if selected else tone.line)))
            painter.setBrush(QColor(tone.selected if selected else tone.field))
            painter.drawRoundedRect(QRectF(left + 1, 36, max(right - left - 2, 2), 81), 6, 6)
            if selected and right - left > 20:
                for x in (left + 5, right - 7):
                    painter.fillRect(QRectF(x, 58, 3, 35), QColor(tone.accent))
            visible_left = max(left, -4)
            if right - visible_left > 50:
                glyph("film", tone.accent, 17).paint(
                    painter, round(visible_left + 14), 50, 17, 17,
                )
                painter.setPen(QColor(tone.fg))
                label = painter.fontMetrics().elidedText(
                    Path(clip.path).name, Qt.TextElideMode.ElideRight,
                    max(0, round(min(right, self.width()) - visible_left - 51)),
                )
                painter.drawText(round(visible_left + 41), 65, label)
                painter.setPen(QColor(tone.muted))
                painter.drawText(round(visible_left + 14), 99, timecode(clip.length))
        for start, finish in self.cuts:
            left, right = self.x_at(start), self.x_at(finish)
            painter.fillRect(QRectF(left, 108, right - left, 7), QColor(tone.cut))
        if self.drag and self.drag[0] == "move" and self.drag[3] != self.drag[1]:
            x = self.x_at(self.offsets[self.drag[3]])
            painter.fillRect(QRectF(x - 1, 34, 3, 85), QColor(tone.accent))
        x = self.x_at(self.playhead)
        painter.fillRect(QRectF(x - .5, 4, 1.5, 123), QColor(tone.accent))
        painter.setPen(Qt.PenStyle.NoPen)
        painter.setBrush(QColor(tone.accent))
        painter.drawRoundedRect(QRectF(x - 5, 1, 10, 6), 2, 2)

    def edge_at(self, x, index):
        left, right = self.x_at(self.offsets[index]), self.x_at(self.offsets[index + 1])
        if right - left < 12:
            return None
        edge = min((abs(left - x), True), (abs(right - x), False))
        return edge[1] if edge[0] <= 9 else None

    def mousePressEvent(self, event):
        if event.button() != Qt.MouseButton.LeftButton or not self.clips:
            return
        x, y = event.position().x(), event.position().y()
        timestamp = self.time_at(x)
        if y < self.ruler_height:
            self.drag = ("seek",)
            self.seeked.emit(timestamp)
        elif 36 <= y <= 117 and self.x_at(self.duration) + 9 >= x:
            index = self.index_at(timestamp)
            edge = self.edge_at(x, index)
            self.selected.emit(index)
            if edge is None:
                self.drag = ("move", index, x, index)
                self.seeked.emit(timestamp)
            else:
                self.drag = ("trim", index, x, edge, self.pps)
        self.update()

    def mouseMoveEvent(self, event):
        x, y = event.position().x(), event.position().y()
        if self.drag:
            kind, *data = self.drag
            if kind == "seek":
                self.seeked.emit(self.time_at(x))
            elif kind == "move":
                index, origin, _ = data
                target = self.index_at(self.time_at(x)) if abs(x - origin) > 8 else index
                self.drag = (kind, index, origin, target)
                self.update()
            else:
                index, origin, left, pps = data
                self.trimmed.emit(index, left, (x - origin) / pps, False)
        elif self.clips and 36 <= y <= 117 and x <= self.x_at(self.duration) + 9:
            edge = self.edge_at(x, self.index_at(self.time_at(x)))
            self.setCursor(Qt.CursorShape.SizeHorCursor if edge is not None else Qt.CursorShape.OpenHandCursor)
        else:
            self.unsetCursor()

    def mouseReleaseEvent(self, event):
        if event.button() != Qt.MouseButton.LeftButton or self.drag is None:
            return
        (kind, *data), self.drag = self.drag, None
        if kind == "move" and data[0] != data[2]:
            self.reordered.emit(data[0], data[2])
        elif kind == "trim":
            index, origin, left, pps = data
            self.trimmed.emit(index, left, (event.position().x() - origin) / pps, True)
        elif kind == "seek":
            self.seeked.emit(self.time_at(event.position().x()))
        self.update()

    def wheelEvent(self, event):
        delta = event.angleDelta()
        if event.modifiers() & Qt.KeyboardModifier.ShiftModifier or delta.x():
            self.scroll(self.offset - (delta.x() or delta.y()) / self.pps)
        elif delta.y():
            self.zoom(1.25 ** (delta.y() / 120), event.position().x())
        else:
            event.ignore()
