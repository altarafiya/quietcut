#!/usr/bin/python3
"""QuietCut's Linux editor. Playback uses Qt, and edits render through the Rust engine."""
from bisect import bisect_right
from dataclasses import dataclass, replace
from pathlib import Path
import json
import os
import shutil
import sys
import tempfile
from time import monotonic

from PyQt6.QtCore import QProcess, QSettings, Qt, QTimer, QUrl
from PyQt6.QtGui import QDesktopServices, QIcon, QKeySequence, QShortcut
from PyQt6.QtMultimedia import QAudioOutput, QMediaPlayer, QVideoFrame
from PyQt6.QtWidgets import (
    QApplication, QFileDialog, QFrame, QHBoxLayout, QMainWindow, QMessageBox,
    QProgressBar, QScrollBar, QVBoxLayout, QWidget,
)

from interface import DARK, LIGHT, ExportDialog, Preview, apply_theme, button, glyph, label, readable_size, timecode
from timeline import Timeline

VERSION = "1.8.0"
REVISION = "5"
CLI = os.environ.get("QUIETCUT_CLI") or shutil.which("quietcut-cli") or "/usr/bin/quietcut-cli"
VIDEO_FILTER = "Videos (*.mp4 *.mkv *.mov *.webm *.avi *.m4v *.ts *.mts *.m2ts *.wmv);;All files (*)"
READY = (QMediaPlayer.MediaStatus.LoadedMedia, QMediaPlayer.MediaStatus.BufferedMedia,
         QMediaPlayer.MediaStatus.BufferingMedia, QMediaPlayer.MediaStatus.EndOfMedia)


@dataclass(frozen=True)
class Clip:
    path: str
    source_length: float
    fps: float
    size: int
    start: float = 0.
    end: float = 0.
    audio_tracks: int = 0

    @property
    def length(self):
        return self.end - self.start

    def for_render(self):
        return dict(path=self.path, start=self.start, end=self.end)


@dataclass(frozen=True)
class Span:
    clip: int
    timeline_start: float
    start: float
    end: float

    @property
    def timeline_end(self):
        return self.timeline_start + self.end - self.start


def app_icon():
    source = Path(__file__).resolve().parents[1] / "assets/quietcut.png"
    installed = Path("/usr/share/icons/hicolor/256x256/apps/quietcut.png")
    return QIcon(str(source if source.is_file() else installed))


class Job(QProcess):
    def __init__(self, parent, args, finished, progress=None):
        super().__init__(parent)
        self.on_finish = finished
        self.on_progress = progress
        self.buffer = bytearray()
        self.result = None
        self.cancelled = False
        self.readyReadStandardOutput.connect(self.drain)
        self.finished.connect(self.completed)
        self.errorOccurred.connect(self.failed)
        self.setProgram(CLI)
        self.setArguments(args)

    def drain(self):
        self.buffer.extend(bytes(self.readAllStandardOutput()))
        while b"\n" in self.buffer:
            line, _, self.buffer = self.buffer.partition(b"\n")
            try:
                result = json.loads(line)
            except ValueError:
                continue
            if "progress" in result:
                if self.on_progress and not self.cancelled:
                    self.on_progress(result["progress"], result["status"])
            else:
                self.result = result

    def completed(self, code, status):
        self.drain()
        error = bytes(self.readAllStandardError()).decode(errors="replace").strip()
        self.on_finish(self.result if code == 0 else None, error, self.cancelled)
        self.deleteLater()

    def failed(self, error):
        if error == QProcess.ProcessError.FailedToStart:
            self.on_finish(None, self.errorString(), self.cancelled)
            self.deleteLater()

    def cancel(self):
        self.cancelled = True
        self.write(b"c")


class Editor(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("QuietCut 1.8")
        self.setWindowIcon(app_icon())
        self.resize(1100, 740)
        self.setMinimumSize(830, 530)
        self.setAcceptDrops(True)
        self.prefs = QSettings("QuietCut", "QuietCut")
        self.dark = self.prefs.value("dark", True, type=bool)
        self.clips = []
        self.selection = -1
        self.history = []
        self.original_trim = None
        self.playhead = 0.
        self.plan = None
        self.spans = []
        self.span_starts = []
        self.active_span = -1
        self.playing = False
        self.pending_media = None
        self.seek_guard = None
        self.preview_path = None
        self.in_preview = False
        self.last_export = None
        self.job = None
        self.plan_job = None
        self.plan_generation = 0
        self.plan_status = ""
        self.play_after_analysis = False
        self.scratch = tempfile.TemporaryDirectory(prefix="quietcut-")

        page = QWidget()
        page.setObjectName("editor")
        layout = QVBoxLayout(page)
        layout.setContentsMargins(24, 18, 24, 24)
        layout.setSpacing(0)
        self.setCentralWidget(page)

        header = QHBoxLayout()
        header.setSpacing(10)
        icon = label("")
        icon.setPixmap(self.windowIcon().pixmap(29, 29))
        header.addWidget(icon)
        header.addWidget(label("QuietCut 1.8", "title"))
        header.addStretch()
        self.new_button = button("New project", self.new_project, "plus")
        self.import_button = button("Import", self.choose_files, "import")
        self.export_button = button("Export", self.open_export, "export", primary=True)
        self.theme_button = button("", self.toggle_theme, "theme")
        self.theme_button.setAccessibleName("Toggle light or dark theme")
        self.theme_button.setToolTip("Toggle light or dark theme")
        for control, width in ((self.new_button, 140), (self.import_button, 112),
                               (self.export_button, 124), (self.theme_button, 45)):
            control.setMinimumWidth(width)
            control.setFixedHeight(38)
            header.addWidget(control)
        layout.addLayout(header)
        layout.addSpacing(12)
        rule = QFrame()
        rule.setObjectName("rule")
        rule.setFixedHeight(1)
        layout.addWidget(rule)
        layout.addSpacing(9)

        self.preview = Preview(self.colors)
        layout.addWidget(self.preview, 1)
        self.player = QMediaPlayer(self)
        self.audio = QAudioOutput(self)
        self.player.setAudioOutput(self.audio)
        self.player.setVideoSink(self.preview.sink)
        self.player.positionChanged.connect(self.position_changed)
        self.player.mediaStatusChanged.connect(self.media_status)
        self.player.errorOccurred.connect(self.media_error)
        self.media_timer = QTimer(self, singleShot=True)
        self.media_timer.timeout.connect(self.apply_media)
        self.scrub_timer = QTimer(self, singleShot=True, interval=22)
        self.scrub_timer.timeout.connect(self.apply_scrub)
        self.scrub_target = None

        transport = QHBoxLayout()
        transport.setSpacing(8)
        self.play_button = button("Play", self.toggle_play, "play")
        self.play_button.setMinimumWidth(94)
        transport.addWidget(self.play_button)
        self.clock = label("")
        self.clock.setMinimumWidth(132)
        transport.addWidget(self.clock)
        transport.addStretch()
        self.split_button = button("Split", self.split, "split")
        self.delete_button = button("Delete", self.delete_clip, "delete")
        self.undo_button = button("Undo", self.undo, "undo")
        self.left_button = button("Left", lambda: self.move_clip(self.selection, self.selection - 1), "left")
        self.right_button = button("Right", lambda: self.move_clip(self.selection, self.selection + 1), "right")
        for control in (self.split_button, self.delete_button, self.undo_button, self.left_button, self.right_button):
            transport.addWidget(control)
        layout.addSpacing(16)
        layout.addLayout(transport)
        layout.addSpacing(16)

        self.timeline = Timeline(self.colors)
        self.timeline.seeked.connect(self.seek)
        self.timeline.selected.connect(self.select)
        self.timeline.trimmed.connect(self.trim)
        self.timeline.reordered.connect(self.move_clip)
        layout.addWidget(self.timeline)
        self.scrollbar = QScrollBar(Qt.Orientation.Horizontal)
        self.scrollbar.setAccessibleName("Scroll timeline")
        self.scrollbar.valueChanged.connect(lambda value: self.timeline.scroll(value / 1000))
        self.timeline.view_changed.connect(self.sync_scroll)
        layout.addSpacing(7)
        layout.addWidget(self.scrollbar)
        layout.addSpacing(5)
        self.progress = QProgressBar()
        self.progress.setFixedHeight(4)
        self.progress.setTextVisible(False)
        self.progress.hide()
        layout.addWidget(self.progress)
        layout.addSpacing(38)
        footer = QHBoxLayout()
        footer.setSpacing(6)
        self.status = label("Import or drop videos", "muted")
        footer.addWidget(self.status, 1)
        self.cancel_button = button("Cancel", self.cancel_render)
        self.cancel_button.hide()
        footer.addWidget(self.cancel_button)
        self.open_button = button("Open folder", self.open_folder, "folder")
        self.open_button.hide()
        footer.addWidget(self.open_button)
        for text, name, action in (
            ("−", "Zoom out", lambda: self.timeline.zoom(.8)),
            ("+", "Zoom in", lambda: self.timeline.zoom(1.25)),
            ("Fit", "Fit timeline", self.timeline.fit),
        ):
            control = button(text, action)
            control.setAccessibleName(name)
            control.setToolTip(name)
            control.setMinimumWidth(40)
            footer.addWidget(control)
        layout.addLayout(footer)

        self.panel = ExportDialog(self)
        self.panel.setWindowIcon(self.windowIcon())
        self.analysis_key = self.silence_settings()
        self.panel.changed.connect(self.options_changed)
        self.plan_timer = QTimer(self, singleShot=True, interval=220)
        self.plan_timer.timeout.connect(self.prepare_plan)
        self.shortcuts = []
        for key, action in (
            ("Ctrl+N", self.new_project), ("Ctrl+O", self.choose_files), ("Ctrl+B", self.split),
            ("Ctrl+Z", self.undo), ("Delete", self.delete_clip), ("Space", self.toggle_play),
            ("Left", lambda: self.step(-1)), ("Right", lambda: self.step(1)),
            ("Home", lambda: self.seek(0.)), ("End", lambda: self.seek(self.timeline.duration)),
        ):
            self.shortcuts.append(QShortcut(QKeySequence(key), self, activated=action))
        self.refresh_theme()
        self.refresh_controls()
        self.set_playhead(0.)

    def colors(self):
        return DARK if self.dark else LIGHT

    def refresh_theme(self):
        apply_theme(self, self.colors())
        apply_theme(self.panel, self.colors())
        self.timeline.update()
        self.preview.update()
        self.set_playing(self.playing)

    def toggle_theme(self):
        self.dark = not self.dark
        self.prefs.setValue("dark", self.dark)
        self.refresh_theme()

    def refresh_controls(self):
        idle, has_clip = self.job is None, bool(self.clips)
        self.timeline.setEnabled(idle)
        for control in (self.new_button, self.import_button):
            control.setEnabled(idle)
        for control in (self.play_button, self.export_button, self.split_button):
            control.setEnabled(idle and has_clip)
        self.undo_button.setEnabled(idle and bool(self.history))
        self.delete_button.setEnabled(idle and self.selection >= 0)
        self.left_button.setEnabled(idle and self.selection > 0)
        self.right_button.setEnabled(idle and 0 <= self.selection < len(self.clips) - 1)
        self.panel.save.setEnabled(idle and has_clip)
        self.panel.preview_button.setEnabled(idle and has_clip)
        self.panel.new.setVisible(self.last_export is not None and idle)
        self.panel.open.setVisible(self.last_export is not None and idle)
        self.open_button.setVisible(self.last_export is not None and idle)

    def sync_scroll(self):
        self.scrollbar.blockSignals(True)
        self.scrollbar.setRange(0, min(2_147_483_647, round(self.timeline.max_offset * 1000)))
        self.scrollbar.setPageStep(max(1, min(2_147_483_647, round(self.timeline.width() * 1000 / self.timeline.pps))))
        self.scrollbar.setValue(min(2_147_483_647, round(self.timeline.offset * 1000)))
        self.scrollbar.blockSignals(False)

    def select(self, index):
        self.selection = index
        self.timeline.selection = index
        self.timeline.update()
        self.refresh_controls()

    def snapshot(self):
        self.history.append((self.clips.copy(), self.selection, self.playhead))
        del self.history[:-50]

    def edited(self, playhead=None):
        self.pause()
        self.in_preview = False
        self.timeline.set_clips(self.clips)
        self.last_export = None
        self.panel.progress.hide()
        self.panel.status.clear()
        self.select(min(self.selection, len(self.clips) - 1))
        self.invalidate_plan()
        self.seek(self.playhead if playhead is None else playhead)
        self.refresh_controls()

    def undo(self):
        if self.job or not self.history:
            return
        self.clips, self.selection, playhead = self.history.pop()
        self.edited(playhead)

    def new_project(self):
        if self.job or not self.clips:
            return
        answer = QMessageBox.question(
            self, "New project", "Clear the current timeline?",
            defaultButton=QMessageBox.StandardButton.No,
        )
        if answer == QMessageBox.StandardButton.Yes:
            self.clips = []
            self.history.clear()
            self.panel.hide()
            self.panel.name.clear()
            self.edited(0.)
            self.status.setText("Import or drop videos")

    def split(self):
        if self.job or not self.clips:
            return
        index = self.timeline.index_at(self.playhead)
        clip = self.clips[index]
        at = round((clip.start + self.playhead - self.timeline.offsets[index]) * clip.fps) / clip.fps
        if min(at - clip.start, clip.end - at) < .9 / clip.fps:
            return
        self.snapshot()
        self.clips[index:index + 1] = [replace(clip, end=at), replace(clip, start=at)]
        self.selection = index + 1
        self.edited(self.timeline.offsets[index] + at - clip.start)

    def trim(self, index, left, delta, finished):
        if self.original_trim is None:
            self.original_trim = (self.clips.copy(), self.selection, self.playhead)
            self.pause()
        original = self.original_trim
        clip = original[0][index]
        bound = clip.start if left else clip.end
        snapped = round((bound + delta) * clip.fps) / clip.fps
        if left:
            change = replace(clip, start=max(0., min(clip.end - 1 / clip.fps, snapped)))
        else:
            change = replace(clip, end=min(clip.source_length, max(clip.start + 1 / clip.fps, snapped)))
        self.clips[index] = change
        self.timeline.cuts = []
        self.timeline.set_clips(self.clips)
        if finished:
            self.original_trim = None
            if change != clip:
                self.history.append(original)
                del self.history[:-50]
            self.edited(self.timeline.offsets[index])

    def delete_clip(self):
        if self.job or not 0 <= self.selection < len(self.clips):
            return
        self.snapshot()
        offset = self.timeline.offsets[self.selection]
        self.clips.pop(self.selection)
        self.edited(offset)

    def move_clip(self, origin, target):
        if self.job or origin == target or not (0 <= origin < len(self.clips) and 0 <= target < len(self.clips)):
            return
        self.snapshot()
        self.clips.insert(target, self.clips.pop(origin))
        self.selection = target
        self.edited(sum(clip.length for clip in self.clips[:target]))

    def step(self, direction):
        if self.clips:
            clip = self.clips[self.timeline.index_at(self.playhead)]
            self.seek(self.playhead + direction / clip.fps)

    def set_playhead(self, position, scroll=False):
        self.playhead = max(0., min(position, self.timeline.duration))
        self.timeline.playhead = self.playhead
        self.timeline.update()
        self.clock.setText(f"{timecode(self.playhead)} / {timecode(self.timeline.duration)}")
        if scroll:
            self.timeline.ensure_visible()

    def set_playing(self, playing):
        self.playing = playing
        if playing:
            self.scrub_timer.stop()
        self.play_button.setText("Pause" if playing else "Play")
        self.play_button.setIcon(glyph("pause" if playing else "play", self.colors().accent))

    def pause(self):
        self.pending_media = None
        self.media_timer.stop()
        self.scrub_timer.stop()
        self.player.pause()
        self.play_after_analysis = False
        self.set_playing(False)

    def seek(self, timestamp):
        if self.job:
            return
        self.pause()
        self.in_preview = False
        self.set_playhead(timestamp)
        if not self.clips:
            self.player.setSource(QUrl())
            self.preview.frame_changed(QVideoFrame())
            self.preview.message = "Drop videos here"
            return
        index = self.timeline.index_at(self.playhead)
        clip = self.clips[index]
        position = clip.start + self.playhead - self.timeline.offsets[index]
        self.scrub_target = (clip.path, max(clip.start, min(position, clip.end - .5 / clip.fps)))
        self.scrub_timer.start()

    def apply_scrub(self):
        path, position = self.scrub_target
        self.request_media(path, position, False)

    def request_media(self, path, position, play):
        url = QUrl.fromLocalFile(str(path))
        self.pending_media = (max(0, round(position * 1000)), play)
        self.preview.message = ""
        if url != self.player.source():
            self.player.setSource(url)
        if self.player.mediaStatus() in READY:
            self.media_timer.start(0)

    def apply_media(self):
        if self.pending_media is None or self.player.mediaStatus() not in READY:
            return
        (position, play), self.pending_media = self.pending_media, None
        self.seek_guard = (position, monotonic() + .35) if play else None
        if not play:
            self.player.pause()
        self.player.setPosition(position)
        if play:
            self.player.play()

    def media_status(self, status):
        if status in READY and self.pending_media is not None:
            self.media_timer.start(0)
        elif status == QMediaPlayer.MediaStatus.EndOfMedia and self.playing:
            if self.in_preview:
                self.set_playing(False)
            else:
                self.advance()

    def media_error(self, error, message):
        self.pause()
        self.preview.message = "Video preview unavailable"
        self.preview.update()
        self.status.setText(message)

    def position_changed(self, milliseconds):
        if not self.playing or self.pending_media:
            return
        if self.seek_guard:
            expected, until = self.seek_guard
            if monotonic() < until and abs(milliseconds - expected) > 200:
                return
            self.seek_guard = None
        if self.in_preview:
            self.clock.setText(f"{timecode(milliseconds / 1000)} / {timecode(self.player.duration() / 1000)}")
            return
        span = self.spans[self.active_span]
        position = milliseconds / 1000
        if position < span.start - .04:
            return
        if position >= span.end - .008:
            self.advance()
        else:
            self.set_playhead(span.timeline_start + max(0., position - span.start), scroll=True)

    def advance(self):
        index = self.active_span + 1
        if index >= len(self.spans):
            self.pause()
            self.set_playhead(self.timeline.duration, scroll=True)
        else:
            self.play_span(index, self.spans[index].timeline_start)

    def play_span(self, index, timestamp):
        self.active_span = index
        span = self.spans[index]
        position = span.start + max(0., timestamp - span.timeline_start)
        self.set_playhead(max(span.timeline_start, timestamp), scroll=True)
        self.request_media(self.clips[span.clip].path, position, True)

    def toggle_play(self):
        if self.job or not self.clips:
            return
        if self.playing:
            self.pause()
            return
        if self.in_preview:
            self.set_playing(True)
            if self.player.mediaStatus() == QMediaPlayer.MediaStatus.EndOfMedia:
                self.player.setPosition(0)
            self.player.play()
            return
        if self.panel.silence.isChecked() and self.plan is None:
            self.play_after_analysis = not self.plan_status
            self.status.setText(self.plan_status or "Finding silence…")
            return
        if not self.spans:
            self.status.setText("No content remains with these silence settings.")
            return
        start = self.playhead if self.playhead < self.timeline.duration - .01 else 0.
        index = max(0, bisect_right(self.span_starts, start) - 1)
        if self.spans[index].timeline_end <= start + .001:
            index = (index + 1) % len(self.spans)
            start = self.spans[index].timeline_start
        self.set_playing(True)
        self.play_span(index, start)

    def choose_files(self):
        if self.job:
            return
        paths, _ = QFileDialog.getOpenFileNames(self, "Import videos", "", VIDEO_FILTER)
        self.import_paths(paths)

    def import_paths(self, paths):
        if self.job or not paths:
            return
        self.pause()
        self.status.setText("Importing videos…")
        self.start_job(["--probe", *map(os.path.abspath, paths)], self.import_done)

    def import_done(self, result):
        imported, errors = [], []
        for item in result["media"]:
            if "error" in item:
                errors.append(f"{Path(item['path']).name}\n{item['error']}")
            else:
                imported.append(Clip(item["path"], item["duration"], item["fps"], item["size"],
                                     end=item["duration"], audio_tracks=item.get("audio_tracks", 0)))
        if imported:
            self.snapshot()
            had_clips = bool(self.clips)
            self.selection = len(self.clips)
            offset = self.timeline.duration
            self.clips.extend(imported)
            self.edited(offset)
            if not had_clips:
                self.panel.name.setText(f"{Path(imported[0].path).stem} - QuietCut")
            self.status.setText("Finding silence…" if self.plan is None else "Ready")
        if errors:
            QMessageBox.warning(self, "Could not import some files", "\n\n".join(errors))

    def dragEnterEvent(self, event):
        if not self.job and any(url.isLocalFile() for url in event.mimeData().urls()):
            event.acceptProposedAction()

    def dropEvent(self, event):
        self.import_paths([url.toLocalFile() for url in event.mimeData().urls() if url.isLocalFile()])
        event.acceptProposedAction()

    def silence_settings(self):
        if not self.panel.silence.isChecked():
            return None
        settings = self.panel.options()
        return tuple(settings[key] for key in ("threshold", "min_silence", "padding", "adaptive"))

    def options_changed(self):
        analysis = self.silence_settings()
        plan_changed = analysis != self.analysis_key
        was_preview = self.in_preview
        self.analysis_key = analysis
        self.pause()
        self.last_export = None
        self.panel.progress.hide()
        self.panel.status.clear()
        self.refresh_controls()
        if plan_changed:
            self.invalidate_plan()
        else:
            self.invalidate_preview()
            self.update_metrics()
        if self.clips and (plan_changed or was_preview):
            self.seek(self.playhead)

    def invalidate_preview(self):
        if self.preview_path:
            if self.player.source().toLocalFile() == str(self.preview_path):
                self.player.setSource(QUrl())
            self.preview_path.unlink(missing_ok=True)
            self.preview_path = None
        self.in_preview = False

    def invalidate_plan(self):
        self.plan_generation += 1
        self.plan_status = ""
        self.plan_timer.stop()
        if self.plan_job:
            self.plan_job.cancel()
        self.invalidate_preview()
        self.timeline.cuts = []
        if self.clips and self.panel.silence.isChecked():
            self.plan = None
            self.plan_timer.start()
            self.status.setText("Finding silence…")
        else:
            spans = [Span(i, self.timeline.offsets[i], clip.start, clip.end)
                     for i, clip in enumerate(self.clips)]
            self.set_plan(spans, self.timeline.duration)
            if self.clips:
                self.status.setText("Ready")
        self.update_metrics()
        self.timeline.update()

    def project_file(self, name, sample=None):
        project = Path(self.scratch.name) / name
        project.write_text(json.dumps({
            "clips": [clip.for_render() for clip in self.clips] if sample is None else sample,
            "remove_silence": self.panel.silence.isChecked() and sample is None,
            "audio_only": self.panel.audio_only(),
            "settings": self.panel.options(),
        }), encoding="utf-8")
        return project

    def prepare_plan(self):
        if self.job or self.plan_job:
            self.plan_timer.start()
            return
        if not self.clips or not self.panel.silence.isChecked():
            return
        generation = self.plan_generation
        path = self.project_file("analysis.json")

        def finished(result, error, cancelled):
            self.plan_job = None
            if generation != self.plan_generation:
                return
            if result is not None:
                self.set_plan([Span(**item) for item in result["spans"]], result["duration"])
                self.status.setText("Ready" if self.spans else "No content remains with these silence settings.")
                if self.play_after_analysis:
                    self.play_after_analysis = False
                    self.toggle_play()
            elif not cancelled:
                self.plan_status = error or "Could not analyze silence."
                self.status.setText(self.plan_status)
                self.update_metrics()

        def progress(_fraction, status):
            if generation == self.plan_generation:
                self.status.setText(status)

        self.plan_job = Job(self, ["--plan-project", str(path)], finished, progress)
        self.plan_job.start()

    def set_plan(self, spans, duration):
        self.plan, self.spans = duration, spans
        self.span_starts = [span.timeline_start for span in spans]
        cuts = []
        cursor = 0.
        for span in spans:
            if span.timeline_start - cursor > .001:
                cuts.append((cursor, span.timeline_start))
            cursor = span.timeline_end
        if self.timeline.duration - cursor > .001:
            cuts.append((cursor, self.timeline.duration))
        self.timeline.cuts = cuts
        self.timeline.update()
        self.update_metrics()

    def update_metrics(self):
        self.panel.before_time.setText(timecode(self.timeline.duration))
        size = sum(clip.size * clip.length / clip.source_length for clip in self.clips)
        approximate = any(clip.length < clip.source_length for clip in self.clips)
        self.panel.before_size.setText(("≈" if approximate else "") + readable_size(size))
        if self.last_export is None:
            self.show_duration(self.plan)
            self.panel.after_size.setText("—")

    def show_duration(self, duration, exported=False):
        self.panel.duration_title.setText("Exported duration" if exported else "Estimated duration")
        if duration is None:
            self.panel.after_time.setText("—")
            self.panel.delta.setText("Estimate unavailable" if self.plan_status else "Analyzing pauses…")
            return
        self.panel.after_time.setText(timecode(duration))
        total = self.timeline.duration
        removed = total - duration
        if removed > .05:
            self.panel.delta.setText(f"{timecode(removed)} shorter · {removed / total:.0%} removed")
        elif self.panel.silence.isChecked():
            self.panel.delta.setText("No pauses removed")
        else:
            self.panel.delta.setText("Silence removal off")

    def open_export(self):
        if self.clips and not self.job:
            if not self.panel.name.text().strip():
                self.panel.name.setText(f"{Path(self.clips[0].path).stem} - QuietCut")
            self.panel.show()
            self.panel.raise_()
            self.panel.activateWindow()

    def preview_edit(self):
        if self.job or not self.clips:
            return
        sample = None
        if self.panel.sample_preview():
            if self.plan is None:
                self.panel.status.setText(self.plan_status or "Finding silence. Preview will be available shortly.")
                return
            sample = self.preview_sample()
            if not sample:
                self.panel.status.setText("No audio to preview." if self.panel.audio_only() else "No video to preview.")
                return
        self.panel.hide()
        if sample is None and not self.panel.level.isChecked():
            if not self.playing:
                self.toggle_play()
        elif self.preview_path:
            self.play_rendered_preview()
        else:
            self.render(Path(self.scratch.name) / f"preview.{self.panel.extension()}", preview=True, sample=sample)

    def preview_sample(self):
        remaining = 8.
        sample = []
        for span in self.spans:
            clip = self.clips[span.clip]
            if not sample and self.panel.audio_only() and not clip.audio_tracks:
                continue
            end = min(span.end, span.start + remaining)
            if end - span.start < .001:
                break
            sample.append(dict(path=clip.path, start=span.start, end=end))
            remaining -= end - span.start
        return sample

    def play_rendered_preview(self):
        self.pause()
        self.in_preview = True
        self.set_playing(True)
        self.request_media(self.preview_path, 0., True)
        if self.panel.audio_only():
            self.preview.message = "Audio preview"
            self.preview.update()
        preview = "Encoded sample" if self.panel.sample_preview() else "Edited preview"
        self.status.setText(f"{preview} · click the ruler to return to the timeline")

    def export(self):
        if self.job or not self.clips:
            return
        if self.panel.audio_only() and not any(clip.audio_tracks for clip in self.clips):
            self.panel.status.setText("The timeline has no audio tracks to export.")
            return
        ext = self.panel.extension()
        name = self.panel.name.text().strip()
        if not name or name in (".", "..") or "/" in name or "\n" in name or "\r" in name:
            QMessageBox.warning(self.panel, "Invalid filename", "Enter a file name without a path.")
            return
        if name.lower().endswith(f".{ext}") and len(name) > len(ext) + 1:
            name = name[:-(len(ext) + 1)]
        directory = Path(self.clips[0].path).parent
        if self.panel.destination != 0 and self.panel.folder:
            directory = Path(self.panel.folder)
        target = directory / f"{name}.{ext}"
        number = 2
        while target.exists():
            target = directory / f"{name} {number}.{ext}"
            number += 1
        if self.panel.destination == 2:
            media = "audio" if self.panel.audio_only() else "video"
            path, _ = QFileDialog.getSaveFileName(
                self.panel, f"Save {media}", str(target), f"{ext.upper()} {media} (*.{ext})",
                options=QFileDialog.Option.DontConfirmOverwrite,
            )
            if not path:
                return
            target = Path(path)
            if target.suffix.lower() != f".{ext}":
                target = target.with_name(target.name + "." + ext)
            self.panel.folder = str(target.parent)
            self.prefs.setValue("export_dir", self.panel.folder)
        if target.exists():
            QMessageBox.warning(self.panel, "Choose another filename", f"{target.name} already exists.")
            return
        self.render(target, preview=False)

    def render(self, target, preview, sample=None):
        self.pause()
        if self.plan_job:
            self.plan_generation += 1
            self.plan_job.cancel()
            if self.plan is None:
                self.plan_timer.start()
        path = self.project_file("render.json", sample)
        self.render_started = monotonic()
        self.progress.setRange(0, 0)
        self.progress.show()
        self.panel.progress.setRange(0, 0)
        self.panel.progress.show()
        self.panel.set_busy(True)
        self.cancel_button.show()
        self.status.setText("Preparing preview…" if preview else "Preparing export…")

        def done(result):
            output = Path(result["output"])
            if preview:
                self.preview_path = output
                self.panel.progress.hide()
                self.play_rendered_preview()
                return
            self.last_export = output
            self.show_duration(result["duration"], exported=True)
            self.panel.after_size.setText(readable_size(result["size"]))
            self.panel.progress.setRange(0, 100)
            self.panel.progress.setValue(100)
            self.panel.status.setText(f"Done · {timecode(monotonic() - self.render_started)}")
            self.status.setText(f"Saved {output.name}")
            self.refresh_controls()
            QApplication.beep()

        self.start_job(["--render-project", str(path), str(target)], done, self.render_progress)

    def render_progress(self, fraction, message):
        value = max(0, min(100, round(fraction * 100)))
        elapsed = monotonic() - self.render_started
        for progress in (self.progress, self.panel.progress):
            progress.setRange(0, 100)
            progress.setValue(value)
        eta = ""
        if fraction > .25 and elapsed > 2:
            eta = f"\nAbout {timecode(elapsed * (1 - fraction) / fraction)} remaining"
        self.panel.status.setText(f"{value}%  ·  Elapsed {timecode(elapsed)}\n{message}{eta}")
        self.status.setText(message)

    def cancel_render(self):
        if self.job:
            self.job.cancel()
            self.panel.cancel.setEnabled(False)
            self.cancel_button.setEnabled(False)
            self.panel.status.setText("Cancelling…")

    def start_job(self, args, done, progress=None):
        def finished(result, error, cancelled):
            self.job = None
            self.progress.hide()
            self.panel.set_busy(False)
            self.panel.cancel.setEnabled(True)
            self.cancel_button.setEnabled(True)
            self.cancel_button.hide()
            self.refresh_controls()
            if result is not None:
                done(result)
            else:
                self.panel.progress.hide()
                self.panel.status.setText("Cancelled" if cancelled else "")
                self.status.setText("Cancelled" if cancelled else "QuietCut could not finish.")
                if not cancelled:
                    QMessageBox.critical(
                        self.panel if self.panel.isVisible() else self, "QuietCut could not finish",
                        error or "The media engine exited early.",
                    )

        self.job = Job(self, args, finished, progress)
        self.refresh_controls()
        self.job.start()

    def open_folder(self):
        if self.last_export:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(self.last_export.parent)))

    def closeEvent(self, event):
        if self.job:
            self.status.setText("Wait for the current job or cancel it before closing.")
            event.ignore()
            return
        self.plan_timer.stop()
        self.plan_generation += 1
        if self.plan_job:
            job = self.plan_job
            job.cancel()
            if not job.waitForFinished(1200):
                self.status.setText("Stopping silence analysis…")
                event.ignore()
                QTimer.singleShot(300, self.close)
                return
        self.player.stop()
        self.player.setSource(QUrl())
        self.scratch.cleanup()
        event.accept()


def main():
    if sys.argv[1:] == ["--version"]:
        print(f"QuietCut {VERSION}-{REVISION} (Linux)")
        return
    app = QApplication(sys.argv)
    app.setApplicationName("QuietCut")
    app.setDesktopFileName("quietcut")
    app.setWindowIcon(app_icon())
    window = Editor()
    window.show()
    if len(sys.argv) > 1:
        QTimer.singleShot(0, lambda: window.import_paths(sys.argv[1:]))
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
