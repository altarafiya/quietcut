"""Colors, icons, preview surface and the separate QuietCut export window."""
from dataclasses import dataclass
from functools import lru_cache

from PyQt6.QtCore import QEvent, QRectF, QSignalBlocker, QSize, Qt, QTimer, pyqtSignal
from PyQt6.QtGui import QColor, QIcon, QPainter, QPixmap
from PyQt6.QtMultimedia import QVideoFrame, QVideoSink
from PyQt6.QtWidgets import (
    QButtonGroup, QCheckBox, QComboBox, QDialog, QFileDialog, QFrame, QGridLayout, QHBoxLayout,
    QLabel, QLineEdit, QProgressBar, QPushButton, QScrollArea, QSizePolicy, QSlider, QToolTip, QVBoxLayout, QWidget,
)


@dataclass(frozen=True)
class Tone:
    bg: str
    panel: str
    field: str
    fg: str
    muted: str
    line: str
    accent: str
    ink: str
    selected: str
    cut: str


DARK = Tone("#121215", "#19191d", "#222228", "#eeeef3", "#9696a3",
            "#31313b", "#d8d8e3", "#19191f", "#2f2f38", "#744145")
LIGHT = Tone("#f4f4f7", "#ffffff", "#e9e9ef", "#1e1e27", "#6b6b7a",
             "#d2d2dd", "#3a3a4c", "#ffffff", "#dddde7", "#da9999")

PATHS = {
    "plus": '<path d="M12 5v14M5 12h14"/>',
    "import": '<path d="M3 7h7l2 3h9v9H3zM12 12v5m-2.5-2.5h5"/>',
    "export": '<path d="M12 15V3m-4 4 4-4 4 4M5 13v7h14v-7"/>',
    "theme": '<circle cx="12" cy="12" r="8"/><path d="M12 4a8 8 0 0 1 0 16Z" fill="currentColor"/>',
    "play": '<path d="m8 4 12 8-12 8z" fill="currentColor" stroke="none"/>',
    "pause": '<path d="M8 5v14m8-14v14" stroke-width="3"/>',
    "split": '<circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="m8 8 12 12M8 16 20 4"/>',
    "delete": '<path d="M4 6h16m-14 0 1 14h10l1-14M9 6V3h6v3m-5 4v6m4-6v6"/>',
    "undo": '<path d="M4 9h10a5 5 0 1 1 0 10H8M8 5 4 9l4 4"/>',
    "left": '<path d="M20 12H4m6-6-6 6 6 6"/>',
    "right": '<path d="M4 12h16m-6-6 6 6-6 6"/>',
    "film": '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 5v14m10-14v14M3 10h4m-4 4h4m10-4h4m-4 4h4"/>',
    "drop": '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 14 3-3 3 3 3-4 3 4"/>',
    "wave": '<path d="M3 12h2m2-4v8m3-12v16m4-14v12m3-8v4m2-2h2"/>',
    "file": '<path d="M6 3h8l4 4v14H6zM14 3v5h4M9 13h6m-6 4h6"/>',
    "summary": '<path d="M4 19h16M7 15v-4m5 4V5m5 10V8"/>',
    "gentle": '<path d="M4 15c3-7 5-7 8-2s5 5 8-2"/>',
    "balanced": '<path d="M4 16h16M6 16v-4m4 4V7m4 9V5m4 11v-6"/>',
    "tight": '<path d="m4 14 5-7 3 10 4-8 4 5"/>',
    "custom": '<path d="M5 4v16m7-16v16m7-16v16M3 9h4m3 5h4m3-6h4" stroke-width="2"/>',
    "copy": '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V4H4v12h4"/>',
    "lossless": '<path d="M12 3 4 7v6c0 4 4 7 8 8 4-1 8-4 8-8V7zM8 12l3 3 5-6"/>',
    "compact": '<path d="M4 4l6 6m-5 0h5V5m10 15-6-6m5 0h-5v5"/>',
    "ask": '<circle cx="12" cy="12" r="9"/><path d="M9 9a3 3 0 0 1 6 0c0 2-3 2-3 4m0 3h.01"/>',
    "folder": '<path d="M3 7h7l2 3h9v9H3z"/>',
}


@lru_cache(maxsize=160)
def glyph(name, color, size=18, disabled=None):
    result = QIcon()
    colors = [(QIcon.Mode.Normal, color)]
    if disabled:
        colors.append((QIcon.Mode.Disabled, disabled))
    for mode, foreground in colors:
        for scale in (1, 2):
            pixels = size * scale
            svg = (
                f'<svg xmlns="http://www.w3.org/2000/svg" width="{pixels}" height="{pixels}" viewBox="0 0 24 24">'
                f'<g color="{foreground}" fill="none" stroke="{foreground}" stroke-width="1.65" '
                f'stroke-linecap="round" stroke-linejoin="round">{PATHS[name]}</g></svg>'
            )
            pixmap = QPixmap()
            pixmap.loadFromData(svg.encode(), "SVG")
            pixmap.setDevicePixelRatio(scale)
            result.addPixmap(pixmap, mode)
    return result


def themed_icon(widget, name, size=18):
    widget.setProperty("glyph", name)
    widget.setProperty("glyphSize", size)
    widget.setIconSize(QSize(size, size))
    return widget


def button(text, callback, icon=None, primary=False):
    widget = QPushButton(text)
    if icon:
        themed_icon(widget, icon)
    if primary:
        widget.setProperty("primary", True)
    widget.clicked.connect(callback)
    widget.setAutoDefault(False)
    widget.setMinimumHeight(36)
    return widget


def apply_theme(root, tone):
    root.setStyleSheet(f"""
        QWidget {{ color: {tone.fg}; font-size: 14px; }}
        QMainWindow, QDialog, QWidget#editor {{ background: {tone.bg}; }}
        QLabel[role="title"] {{ font-size: 27px; font-weight: 600; }}
        QLabel[role="metric"] {{ font-size: 34px; font-weight: 600; }}
        QLabel[role="iconBadge"] {{ background: {tone.field}; border: 1px solid {tone.line}; border-radius: 6px; }}
        QLabel[role="heading"] {{ font-size: 16px; font-weight: 600; }}
        QLabel[role="strong"] {{ font-weight: 600; }}
        QLabel[role="hint"] {{ color: {tone.muted}; font-size: 12px; }}
        QLabel[role="suffix"] {{ color: {tone.muted}; background: {tone.field}; border: 1px solid {tone.line}; border-radius: 6px; padding: 0px 10px; }}
        QLabel[role="muted"] {{ color: {tone.muted}; }}
        QScrollArea#exportSettings {{ background: transparent; border: none; }}
        QFrame#rule {{ background: {tone.line}; }}
        QFrame#card {{ background: {tone.panel}; border: 1px solid {tone.line}; border-radius: 9px; }}
        QPushButton, QComboBox, QLineEdit {{
            background: {tone.field}; border: 1px solid {tone.line}; border-radius: 7px;
            padding: 0px 12px;
        }}
        QPushButton:hover, QComboBox:hover {{ background: {tone.selected}; border-color: {tone.muted}; }}
        QPushButton:focus, QComboBox:focus, QLineEdit:focus {{ border-color: {tone.accent}; }}
        QPushButton:checked {{ background: {tone.selected}; border-color: {tone.accent}; }}
        QPushButton[primary="true"] {{ background: {tone.accent}; color: {tone.ink}; font-weight: 600; }}
        QPushButton[primary="true"]:hover {{ border-color: {tone.fg}; }}
        QPushButton:disabled, QComboBox:disabled {{ color: {tone.muted}; background: {tone.panel}; border-color: {tone.line}; }}
        QFrame#segments {{ background: {tone.bg}; border: 1px solid {tone.line}; border-radius: 8px; }}
        QPushButton[segment="true"] {{ background: transparent; border-color: transparent; padding: 0 7px; }}
        QPushButton[segment="true"]:hover {{ background: {tone.field}; }}
        QPushButton[segment="true"]:checked {{ background: {tone.selected}; border-color: {tone.muted}; font-weight: 600; }}
        QPushButton[segment="true"]:focus {{ border-color: {tone.accent}; }}
        QComboBox::drop-down {{ border: none; width: 30px; }}
        QComboBox::down-arrow {{ image: none; }}
        QToolTip {{ color: {tone.fg}; background: {tone.field}; border: 1px solid {tone.line}; padding: 5px; }}
        QComboBox QAbstractItemView {{ color: {tone.fg}; background: {tone.field}; selection-background-color: {tone.selected}; border: 1px solid {tone.line}; }}
        QScrollBar:horizontal {{ background: {tone.panel}; height: 14px; border-radius: 5px; }}
        QScrollBar::handle:horizontal {{ background: {tone.selected}; border: 1px solid {tone.line}; border-radius: 5px; min-width: 18px; }}
        QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {{ width: 0px; }}
        QScrollBar::add-page:horizontal, QScrollBar::sub-page:horizontal {{ background: none; }}
        QProgressBar {{ background: {tone.line}; border: none; border-radius: 2px; }}
        QProgressBar::chunk {{ background: {tone.accent}; border-radius: 2px; }}
        QSlider::groove:horizontal {{ height: 4px; background: {tone.line}; }}
        QSlider::handle:horizontal {{ width: 14px; background: {tone.accent}; margin: -5px 0; border-radius: 6px; }}
    """)
    for widget in root.findChildren(QPushButton):
        name = widget.property("glyph")
        if name:
            size = widget.property("glyphSize")
            color = tone.ink if widget.property("primary") else tone.accent
            widget.setIcon(glyph(name, color, size, disabled=tone.muted))
    for mark in root.findChildren(IconMark):
        mark.setPixmap(glyph(mark.name, tone.accent if mark.badge else tone.muted, mark.icon_size).pixmap(mark.icon_size))
    for combo in root.findChildren(Combo):
        combo.update_icons(tone)


def label(text, role=None):
    widget = QLabel(text)
    if role:
        widget.setProperty("role", role)
    return widget


def timecode(seconds):
    tenths = round(max(0., seconds) * 10)
    minutes, rem = divmod(tenths, 600)
    hours, minutes = divmod(minutes, 60)
    return (f"{hours}:{minutes:02}:" if hours else f"{minutes}:") + f"{rem // 10:02}.{rem % 10}"


def readable_size(size):
    if size >= 1024 ** 3:
        return f"{size / 1024 ** 3:.2f} GB"
    if size >= 1024 ** 2:
        return f"{size / 1024 ** 2:.1f} MB"
    return f"{size / 1024:.0f} KB"


class Preview(QWidget):
    def __init__(self, colors):
        super().__init__()
        self.colors = colors
        self.message = "Drop videos here"
        self.frame = QVideoFrame()
        self.sink = QVideoSink(self)
        self.sink.videoFrameChanged.connect(self.frame_changed)
        self.setAccessibleName("Video preview")
        self.setMinimumHeight(120)

    def frame_changed(self, frame):
        self.frame = QVideoFrame(frame)
        self.update()

    def paintEvent(self, event):
        tone = self.colors()
        area = QRectF(self.rect()).adjusted(.5, .5, -.5, -.5)
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        painter.setPen(QColor(tone.line))
        painter.setBrush(QColor("#080809" if self.frame.isValid() else tone.panel))
        painter.drawRoundedRect(area, 10, 10)
        if self.frame.isValid():
            options = QVideoFrame.PaintOptions()
            options.aspectRatioMode = Qt.AspectRatioMode.KeepAspectRatio
            self.frame.paint(painter, area.adjusted(1, 1, -1, -1), options)
        elif self.message:
            middle = self.rect().center()
            glyph("drop", tone.accent, 24).paint(painter, middle.x() - 12, middle.y() - 35, 24, 24)
            font = painter.font()
            font.setPixelSize(18)
            font.setWeight(font.Weight.DemiBold)
            painter.setFont(font)
            painter.setPen(QColor(tone.fg))
            painter.drawText(
                QRectF(0, middle.y() - 3, self.width(), 32),
                Qt.AlignmentFlag.AlignCenter, self.message,
            )


class Switch(QCheckBox):
    """An accessible checkbox with a switch-shaped indicator."""
    def __init__(self, name, colors):
        super().__init__()
        self.colors = colors
        self.setAccessibleName(name)
        self.setFixedSize(70, 28)
        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def hitButton(self, position):
        return self.rect().contains(position)

    def paintEvent(self, event):
        tone = self.colors()
        checked = self.isChecked()
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        painter.setPen(QColor(tone.fg if checked else tone.muted))
        painter.drawText(QRectF(0, 0, 25, 28), Qt.AlignmentFlag.AlignVCenter, "On" if checked else "Off")
        painter.setPen(QColor(tone.accent if self.hasFocus() or checked else tone.line))
        painter.setBrush(QColor(tone.accent if checked and self.isEnabled() else tone.field))
        painter.drawRoundedRect(QRectF(29, 3, 40, 22), 11, 11)
        painter.setPen(Qt.PenStyle.NoPen)
        painter.setBrush(QColor(tone.ink if checked and self.isEnabled() else tone.muted))
        painter.drawEllipse(QRectF(51 if checked else 33, 7, 14, 14))


def rule():
    line = QFrame()
    line.setObjectName("rule")
    line.setFixedHeight(1)
    return line


def card():
    frame = QFrame()
    frame.setObjectName("card")
    layout = QVBoxLayout(frame)
    layout.setContentsMargins(17, 14, 17, 14)
    layout.setSpacing(10)
    return frame, layout


class IconMark(QLabel):
    def __init__(self, name, size, badge=False):
        super().__init__()
        self.name, self.icon_size, self.badge = name, size, badge
        extent = size + 10 if badge else size
        self.setFixedSize(extent, extent)
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        if badge:
            self.setProperty("role", "iconBadge")


class Combo(QComboBox):
    def __init__(self, colors, icons):
        super().__init__()
        self.colors = colors
        self.icons = icons
        self.setIconSize(QSize(16, 16))
        self.view().viewport().installEventFilter(self)

    def eventFilter(self, watched, event):
        if event.type() == QEvent.Type.ToolTip and watched is self.view().viewport():
            index = self.view().indexAt(event.pos())
            tip = index.data(Qt.ItemDataRole.ToolTipRole)
            if tip:
                QToolTip.showText(event.globalPos(), tip, watched, self.view().visualRect(index))
                return True
        return super().eventFilter(watched, event)

    def update_icons(self, tone):
        for index, name in enumerate(self.icons):
            self.setItemIcon(index, glyph(name, tone.accent, 16))

    def paintEvent(self, event):
        super().paintEvent(event)
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        tone = self.colors()
        painter.setPen(QColor(tone.fg if self.isEnabled() else tone.muted))
        x, y = self.width() - 17, self.height() // 2
        painter.drawLine(x - 4, y - 2, x, y + 2)
        painter.drawLine(x, y + 2, x + 4, y - 2)


class ExportDialog(QDialog):
    changed = pyqtSignal()
    HEVC = 4
    RECOMMENDED = 24
    quality_names = ("Fast copy · Fastest", "Lossless · Best quality", "High quality · H.264",
                     "Compact · H.264", "Smaller file · HEVC")
    quality_hints = (
        "Copies the video. Cuts snap to keyframes.",
        "Preserves every retained pixel. Produces large files.",
        "Frame-accurate H.264 with more detail than Compact.",
        "Frame-accurate H.264 with more compression than High quality.",
        "H.265 compression for smaller files. Requires an HEVC-compatible player.",
    )
    quality_tips = (
        "Fastest export. Copies video without re-encoding.\nCuts snap to keyframes and joined clips must have matching formats.\nTurns off Remove silence. Even audio still re-encodes audio.",
        "Best image quality. Retained frames keep their original pixels.\nFrame-accurate cuts, with larger files and more processing.",
        "Best quality among the H.264 presets.\nFrame-accurate cuts with smaller files than Lossless.",
        "Smaller H.264 files than High quality. Frame-accurate cuts with more compression.",
        "Use H.265 for better compression. Preview a short sample at the selected quality. NVIDIA acceleration is used when available.",
    )
    destination_tips = (
        "Save the export in the same folder as the first clip.",
        "Pick a folder to reuse for future exports.",
        "Choose the file name and location each time you export.",
    )

    def __init__(self, editor):
        super().__init__(editor)
        self.editor = editor
        self.prefs = editor.prefs
        self.setWindowTitle("QuietCut · Export")
        self.setMinimumWidth(824)
        self.resize(900, 610)
        self.setModal(False)
        self.folder = self.prefs.value("export_dir", "", type=str)
        self.destination = max(0, min(2, self.prefs.value("destination", 2, type=int)))
        self.quality_note = ""

        root = QVBoxLayout(self)
        root.setContentsMargins(24, 21, 24, 20)
        root.setSpacing(16)
        header = QVBoxLayout()
        header.setSpacing(4)
        self.heading = label("Export video", "title")
        header.addWidget(self.heading)
        header.addWidget(label("Trim pauses, level audio, and save your edit.", "muted"))
        root.addLayout(header)

        columns = QHBoxLayout()
        columns.setSpacing(16)
        root.addLayout(columns, 1)
        self.settings_scroll = QScrollArea()
        self.settings_scroll.setObjectName("exportSettings")
        self.settings_scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.settings_scroll.setWidgetResizable(True)
        self.settings_scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.settings_scroll.setMinimumHeight(320)
        self.settings_content = QWidget()
        left = QVBoxLayout(self.settings_content)
        left.setContentsMargins(0, 0, 0, 0)
        left.setSpacing(12)
        self.settings_scroll.setWidget(self.settings_content)
        columns.addWidget(self.settings_scroll, 1)
        audio, audio_layout = card()
        self.add_heading(audio_layout, "Audio cleanup", "wave")
        self.silence = self.add_switch(audio_layout, "Remove silence", "Shorten pauses while protecting speech.",
                                       "silence", True)
        self.silence.setToolTip("Find and remove pauses while protecting speech. Turning this on switches Fast copy to High quality for accurate cuts.")
        self.silence.toggled.connect(self.silence_changed)
        self.preset_box = QFrame()
        self.preset_box.setObjectName("segments")
        self.preset_box.setAccessibleName("Silence preset")
        preset_row = QHBoxLayout(self.preset_box)
        preset_row.setContentsMargins(3, 3, 3, 3)
        preset_row.setSpacing(3)
        self.preset = QButtonGroup(self)
        for i, (text, icon, tip) in enumerate((
            ("Gentle", "gentle", "Keep the natural pacing. Shortens longer pauses and leaves more space around speech."),
            ("Balanced", "balanced", "A starting point for most recordings. Shortens pauses while leaving room around speech."),
            ("Tight", "tight", "Shorten more pauses for a faster pace. Keeps less space around speech."),
            ("Custom", "custom", "Set the silence threshold, minimum pause length, and padding around speech."),
        )):
            option = button(text, self.update_options)
            option.setToolTip(tip)
            themed_icon(option, icon, 15)
            option.setProperty("segment", True)
            option.setCheckable(True)
            option.setMinimumHeight(29)
            self.preset.addButton(option, i)
            preset_row.addWidget(option, 1)
        self.preset.button(max(0, min(3, self.prefs.value("preset", 1, type=int)))).setChecked(True)
        audio_layout.addWidget(self.preset_box)
        self.custom = QWidget()
        fields = QGridLayout(self.custom)
        fields.setContentsMargins(0, 2, 0, 1)
        fields.setHorizontalSpacing(13)
        fields.setVerticalSpacing(5)
        self.threshold = self.slider(fields, 0, "Threshold", -80, -5, -50, 1, " dB")
        self.minimum = self.slider(fields, 1, "Min. silence", 5, 3000, 50, 100, " s")
        self.padding = self.slider(fields, 2, "Keep edges", 0, 200, 14, 100, " s")
        audio_layout.addWidget(self.custom)
        audio_layout.addWidget(rule())
        self.level = self.add_switch(audio_layout, "Even audio", "Bring quieter and louder sections closer together.", "level", False)
        self.level.setToolTip("Compress loud and quiet passages toward a more consistent volume and limit peaks. Re-encodes audio when enabled.")
        self.level.toggled.connect(self.update_options)
        left.addWidget(audio)

        output, output_layout = card()
        self.add_heading(output_layout, "Output", "file")
        form = QGridLayout()
        form.setContentsMargins(0, 0, 0, 0)
        form.setHorizontalSpacing(13)
        form.setVerticalSpacing(8)
        form.setColumnStretch(1, 1)
        output_layout.addLayout(form)

        self.kind = self.combo(
            ("Video", "Audio only · M4A"), ("film", "wave"),
            ("Export the edited video and its linked audio.", "Export AAC audio only, with the same edits and audio cleanup."),
            "Export type",
        )
        self.kind.setCurrentIndex(int(self.prefs.value("audio_only", False, type=bool)))
        self.kind.currentIndexChanged.connect(self.kind_changed)
        self.add_field(form, 0, "Export", self.kind)
        self.quality = self.combo(
            self.quality_names, ("copy", "lossless", "film", "compact", "compact"),
            self.quality_tips, "Quality",
        )
        quality = max(0, min(self.HEVC, self.prefs.value("quality", 2, type=int)))
        self.quality.setCurrentIndex(2 if quality == 0 and self.silence.isChecked() and not self.audio_only() else quality)
        self.quality.currentIndexChanged.connect(self.quality_changed)
        self.quality_label = self.add_field(form, 1, "Quality", self.quality)
        self.quality_hint = label("", "hint")
        self.quality_hint.setWordWrap(True)
        form.addWidget(self.quality_hint, 2, 1)
        self.compression = QWidget()
        controls = QVBoxLayout(self.compression)
        controls.setContentsMargins(0, 2, 0, 2)
        controls.setSpacing(4)
        value_row = QHBoxLayout()
        value_row.addWidget(label("Compression quality", "strong"))
        value_row.addStretch()
        self.compression_value = label("", "hint")
        value_row.addWidget(self.compression_value)
        self.compression_reset = button("Reset", lambda: self.compression_slider.setValue(self.RECOMMENDED))
        self.compression_reset.setMinimumHeight(24)
        self.compression_reset.setToolTip("Restore the recommended quality of 24.")
        value_row.addWidget(self.compression_reset)
        controls.addLayout(value_row)
        self.compression_slider = QSlider(Qt.Orientation.Horizontal)
        self.compression_slider.setAccessibleName("HEVC quality")
        self.compression_slider.setRange(18, 32)
        self.compression_slider.setInvertedAppearance(True)
        self.compression_slider.setInvertedControls(True)
        self.compression_slider.setValue(self.prefs.value("compression_quality", self.RECOMMENDED, type=int))
        self.compression_slider.setToolTip(
            "Lower values keep more detail. Start at 24, then preview a sample. "
            "The value controls CRF on CPU or CQ on NVIDIA."
        )
        self.compression_slider.valueChanged.connect(self.update_options)
        controls.addWidget(self.compression_slider)
        directions = QHBoxLayout()
        directions.addWidget(label("Smaller file", "hint"))
        directions.addStretch()
        directions.addWidget(label("More detail", "hint"))
        controls.addLayout(directions)
        form.addWidget(self.compression, 3, 1)
        self.dest = self.combo(("Video folder", "Choose folder…", "Ask each time"),
                               ("film", "folder", "ask"), self.destination_tips, "Save to")
        self.dest.setCurrentIndex(self.destination)
        self.dest.activated.connect(self.choose_destination)
        self.add_field(form, 4, "Save to", self.dest)
        self.destination_hint = label("", "hint")
        self.destination_hint.setWordWrap(True)
        form.addWidget(self.destination_hint, 5, 1)

        name_field = QWidget()
        name_row = QHBoxLayout(name_field)
        name_row.setContentsMargins(0, 0, 0, 0)
        name_row.setSpacing(6)
        self.name = QLineEdit()
        self.name.setAccessibleName("File name")
        self.name.setToolTip("Name of the exported video. QuietCut adds the extension for the selected quality.")
        self.name.setMinimumHeight(34)
        name_row.addWidget(self.name, 1)
        self.suffix = label(".mp4", "suffix")
        self.suffix.setFixedHeight(34)
        name_row.addWidget(self.suffix)
        self.add_field(form, 6, "File name", name_field, self.name)
        left.addWidget(output)
        left.addStretch()

        summary, summary_layout = card()
        summary.setFixedWidth(245)
        columns.addWidget(summary, alignment=Qt.AlignmentFlag.AlignTop)
        self.add_heading(summary_layout, "Export summary", "summary")
        summary_layout.addSpacing(5)
        self.duration_title = label("Estimated duration", "hint")
        summary_layout.addWidget(self.duration_title)
        self.after_time = label("—", "metric")
        summary_layout.addWidget(self.after_time)
        self.delta = label("", "hint")
        self.delta.setWordWrap(True)
        summary_layout.addWidget(self.delta)
        summary_layout.addSpacing(3)
        summary_layout.addWidget(rule())
        self.before_time = self.summary_row(summary_layout, "Original duration")
        self.before_size = self.summary_row(summary_layout, "Original size")
        self.format = self.summary_row(summary_layout, "Format")
        self.after_size = self.summary_row(summary_layout, "Export size")
        self.after_size.setToolTip("The final file size is available after export.")
        self.progress = QProgressBar()
        self.progress.setFixedHeight(6)
        self.progress.setTextVisible(False)
        self.progress.hide()
        summary_layout.addWidget(self.progress)
        self.status = label("", "hint")
        self.status.setWordWrap(True)
        summary_layout.addWidget(self.status)
        self.open = button("Open folder", editor.open_folder, "folder")
        self.open.hide()
        summary_layout.addWidget(self.open)
        self.new = button("New project", editor.new_project, "plus")
        self.new.hide()
        summary_layout.addWidget(self.new)
        summary_layout.addStretch()

        root.addWidget(rule())
        actions = QHBoxLayout()
        actions.setSpacing(8)
        self.back = button("Back", self.hide, "left")
        actions.addWidget(self.back)
        actions.addStretch()
        self.cancel = button("Cancel export", editor.cancel_render)
        self.cancel.hide()
        actions.addWidget(self.cancel)
        self.preview_button = button("Preview", editor.preview_edit, "play")
        self.preview_button.setMinimumWidth(110)
        self.preview_button.setToolTip("Play the edit with the selected silence and audio settings before exporting.")
        actions.addWidget(self.preview_button)
        self.save = button("Export video", editor.export, "export", primary=True)
        self.save.setMinimumWidth(150)
        self.save.setToolTip("Render the edited timeline to the selected file.")
        actions.addWidget(self.save)
        root.addLayout(actions)

        self.update_options(emit=False)
        self.choose_destination(self.destination, prompt=False)

    def add_heading(self, layout, text, icon):
        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(IconMark(icon, 16, badge=True))
        row.addWidget(label(text, "heading"), 1)
        layout.addLayout(row)

    def add_switch(self, layout, title, hint, setting, default):
        row = QHBoxLayout()
        row.setSpacing(12)
        copy = QVBoxLayout()
        copy.setSpacing(2)
        copy.addWidget(label(title, "strong"))
        copy.addWidget(label(hint, "hint"))
        row.addLayout(copy, 1)
        switch = Switch(title, self.editor.colors)
        switch.setChecked(self.prefs.value(setting, default, type=bool))
        row.addWidget(switch)
        layout.addLayout(row)
        return switch

    def combo(self, items, icons, tips, accessible_name):
        control = Combo(self.editor.colors, icons)
        control.addItems(items)
        for index, tip in enumerate(tips):
            control.setItemData(index, tip, Qt.ItemDataRole.ToolTipRole)
        control.setAccessibleName(accessible_name)
        control.setMinimumHeight(34)
        control.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        return control

    def add_field(self, layout, row, name, widget, buddy=None):
        title = label(name, "strong")
        title.setBuddy(buddy or widget)
        layout.addWidget(title, row, 0)
        layout.addWidget(widget, row, 1)
        return title

    def slider(self, layout, row, text, low, high, default, scale, unit):
        title, display = label(text, "hint"), label("", "hint")
        slider = QSlider(Qt.Orientation.Horizontal)
        slider.setAccessibleName(text)
        title.setBuddy(slider)
        slider.setRange(low, high)
        slider.setValue(self.prefs.value(text, default, type=int))
        layout.addWidget(title, row, 0)
        layout.addWidget(slider, row, 1)
        display.setMinimumWidth(57)
        display.setAlignment(Qt.AlignmentFlag.AlignRight)
        layout.addWidget(display, row, 2)

        def updated(value):
            display.setText(f"{value}{unit}" if scale == 1 else f"{value / scale:.2f}{unit}")
            self.prefs.setValue(text, value)

        updated(slider.value())
        slider.valueChanged.connect(updated)
        slider.valueChanged.connect(self.update_options)
        return slider

    def summary_row(self, layout, heading):
        row = QHBoxLayout()
        row.addWidget(label(heading, "hint"))
        row.addStretch()
        value = label("—", "strong")
        row.addWidget(value)
        layout.addLayout(row)
        return value

    def audio_only(self):
        return self.kind.currentIndex() == 1

    def sample_preview(self):
        return self.audio_only() or self.quality.currentIndex() == self.HEVC

    def kind_changed(self, _kind):
        self.silence_changed(self.silence.isChecked())

    def quality_changed(self, quality):
        self.quality_note = ""
        if quality == 0 and self.silence.isChecked() and not self.audio_only():
            with QSignalBlocker(self.silence):
                self.silence.setChecked(False)
            self.quality_note = "Remove silence was turned off."
        self.update_options()

    def silence_changed(self, enabled):
        self.quality_note = ""
        if enabled and self.quality.currentIndex() == 0 and not self.audio_only():
            with QSignalBlocker(self.quality):
                self.quality.setCurrentIndex(2)
            self.quality_note = "Switched to High quality for accurate silence cuts."
        self.update_options()

    def update_options(self, _value=None, *, emit=True):
        silence = self.silence.isChecked()
        preset = self.preset.checkedId()
        quality = self.quality.currentIndex()
        audio = self.audio_only()
        was_visible = tuple(not control.isHidden() for control in (self.custom, self.quality, self.compression))
        custom, compress = silence and preset == 3, not audio and quality == self.HEVC
        self.preset_box.setVisible(silence)
        self.custom.setVisible(custom)
        self.quality_label.setVisible(not audio)
        self.quality.setVisible(not audio)
        self.compression.setVisible(compress)
        if emit and self.isVisible() and was_visible != (custom, not audio, compress):
            QTimer.singleShot(0, self.fit_height)
        value = self.compression_slider.value()
        self.compression_value.setText(f"{value} · Recommended" if value == self.RECOMMENDED else str(value))
        self.compression_reset.setEnabled(value != self.RECOMMENDED)
        hint = "AAC at 192 kb/s per track. Keeps your edits and audio cleanup." if audio else self.quality_hints[quality]
        if not audio and quality == 0 and self.level.isChecked():
            hint += " Even audio re-encodes audio."
        if self.quality_note:
            hint = f"{hint} {self.quality_note}"
        self.quality_hint.setText(hint)
        self.quality.setToolTip(self.quality_tips[quality])
        title = "Export audio" if audio else "Export video"
        self.heading.setText(title)
        self.save.setText(title)
        sample = self.sample_preview()
        self.preview_button.setText("Preview sample" if sample else "Preview")
        self.preview_button.setToolTip(
            "Encode and play the first eight seconds at these settings." if sample
            else "Play the edit with the selected silence and audio settings before exporting."
        )
        extension = self.extension()
        self.suffix.setText("." + extension)
        self.format.setText(extension.upper())
        for key, value in (("silence", silence), ("preset", preset), ("quality", quality),
                           ("level", self.level.isChecked()), ("audio_only", audio),
                           ("compression_quality", value)):
            self.prefs.setValue(key, value)
        if emit:
            self.changed.emit()

    def showEvent(self, event):
        super().showEvent(event)
        QTimer.singleShot(0, self.fit_height)

    def fit_height(self):
        self.layout().activate()
        chrome = self.height() - self.settings_scroll.viewport().height()
        desired = chrome + self.settings_content.sizeHint().height()
        self.resize(self.width(), min(desired, self.screen().availableGeometry().height() - 52))

    def extension(self):
        if self.audio_only():
            return "m4a"
        return "mp4" if self.quality.currentIndex() >= 2 else "mkv"

    def options(self):
        preset = self.preset.checkedId()
        if preset == 3:
            values = self.threshold.value(), self.minimum.value() / 100, self.padding.value() / 100
        else:
            values = ((-55, .8, .18), (-50, .5, .14), (-50, .3, .12))[preset]
        return dict(threshold=values[0], min_silence=values[1], padding=values[2],
                    quality=self.quality.currentIndex(), compression_quality=self.compression_slider.value(),
                    level=self.level.isChecked(),
                    dark=self.editor.dark, gpu=True, adaptive=preset != 3)

    def choose_destination(self, destination, prompt=True):
        if destination == 1 and prompt:
            folder = QFileDialog.getExistingDirectory(self, "Choose export folder", self.folder)
            if not folder:
                self.dest.setCurrentIndex(self.destination)
                return
            self.folder = folder
            self.prefs.setValue("export_dir", folder)
        self.destination = destination
        self.dest.setCurrentIndex(destination)
        self.prefs.setValue("destination", destination)
        self.dest.setToolTip(self.destination_tips[destination])
        self.destination_hint.setText((
            "Save beside the first clip.", self.folder or "Select an output folder.", "Choose a location when you export.",
        )[destination])

    def set_busy(self, busy):
        for control in (self.silence, self.preset_box, self.custom, self.level, self.name,
                        self.save, self.preview_button, self.kind, self.quality, self.compression, self.dest):
            control.setEnabled(not busy)
        self.cancel.setVisible(busy)
