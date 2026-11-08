#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]
mod export_ui;
mod live_audio;
mod preview;
mod ui;
use quietcut::plan::{PreviewPlan, PreviewSpan};
use quietcut::workflow::{ProgressEstimate, proposed_output};
use quietcut::{engine::*, timeline::*};
use std::{
    ffi::OsString,
    fs,
    mem::{size_of, zeroed},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use ui::*;
const EVENT: u32 = WM_APP + 1;
const IMPORT: usize = 101;
const THEME: usize = 102;
const PLAY: usize = 103;
const SPLIT: usize = 104;
const DELETE: usize = 105;
const UNDO: usize = 106;
const LEFT: usize = 107;
const RIGHT: usize = 108;
const ZOOM_IN: usize = 109;
const ZOOM_OUT: usize = 110;
const FIT: usize = 111;
const EXPORT: usize = 112;
const CANCEL: usize = 113;
const OPEN: usize = 114;
const SCROLL: usize = 115;
const NEW_PROJECT: usize = 116;
const CLEANUP_DIALOG: u32 = WM_APP + 21;
fn settings_path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
        .join("QuietCut/settings-v18.json")
}
fn load_settings() -> Settings {
    fs::read(settings_path())
        .or_else(|_| fs::read(settings_path().with_file_name("settings-v17.json")))
        .or_else(|_| fs::read(settings_path().with_file_name("settings-v16.json")))
        .or_else(|_| fs::read(settings_path().with_file_name("settings-v15-preview.json")))
        .or_else(|_| fs::read(settings_path().with_file_name("settings-v12.json")))
        .ok()
        .and_then(|s| serde_json::from_slice::<Settings>(&s).ok())
        .filter(|s| s.validate().is_ok())
        .map(|mut s| {
            if (s.threshold == -28. && s.min_silence == 0.25)
                || (s.threshold == -35. && s.min_silence == 0.5)
                || (s.threshold == -45. && s.min_silence == 0.8)
            {
                s.threshold = if s.min_silence >= 0.75 { -55. } else { -50. };
                s.padding = if s.min_silence >= 0.75 { 0.18 } else { 0.14 };
                if s.min_silence == 0.25 {
                    s.min_silence = 0.3;
                    s.padding = 0.12;
                }
                s.adaptive = true;
            }
            s.exact_silence(true);
            s
        })
        .unwrap_or_else(|| Settings {
            quality: 2,
            ..Settings::default()
        })
}
fn save_settings(s: &Settings) {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a.to_string_lossy().ends_with("-test"))
    {
        return;
    }
    let p = settings_path();
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(v) = serde_json::to_vec_pretty(s) {
        let _ = fs::write(p, v);
    }
}
enum Event {
    Imported(Vec<Media>, Vec<String>),
    Progress(f64, String),
    Exported(Result<Exported>),
    Prepared(u64, Result<PreviewPlan>),
    Preparing(u64, String),
}
#[derive(Clone)]
enum Drag {
    None,
    Scrub,
    Clip {
        index: usize,
        x: i32,
        target: usize,
        moved: bool,
    },
    Trim {
        index: usize,
        left: bool,
        x: i32,
        original: Timeline,
    },
}
struct App {
    ui: Option<Ui>,
    timeline: Timeline,
    history: Vec<Timeline>,
    selected: Option<usize>,
    playhead: f64,
    scroll: f64,
    pps: f64,
    drag: Drag,
    preview: Option<preview::Preview>,
    video: HWND,
    preview_index: Option<usize>,
    playing: bool,
    last_seek: Instant,
    settings: Settings,
    silence: bool,
    tools: Result<Tools>,
    busy: bool,
    cancel: Cancel,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    status: String,
    progress: f64,
    output: Option<Exported>,
    export_name: Option<String>,
    export_started: Option<Instant>,
    export_elapsed: f64,
    export_estimate: ProgressEstimate,
    export_refreshed: Instant,
    dialog: HWND,
    close_pending: bool,
    prepared: Option<PreviewPlan>,
    plan_ready: bool,
    analysis_generation: u64,
    analysis_cancel: Cancel,
    analysis_due: Option<Instant>,
    audio: Option<live_audio::LiveAudio>,
    audio_next: Option<live_audio::LiveAudio>,
    play_span: Option<usize>,
    video_running: bool,
    last_sync: Instant,
}
impl App {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            ui: None,
            timeline: Timeline::default(),
            history: vec![],
            selected: None,
            playhead: 0.,
            scroll: 0.,
            pps: 1.,
            drag: Drag::None,
            preview: None,
            video: null_mut(),
            preview_index: None,
            playing: false,
            last_seek: Instant::now(),
            settings: load_settings(),
            silence: true,
            tools: Tools::find(),
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            status: "Import or drop videos".into(),
            progress: 0.,
            output: None,
            export_name: None,
            export_started: None,
            export_elapsed: 0.,
            export_estimate: ProgressEstimate::default(),
            export_refreshed: Instant::now(),
            dialog: null_mut(),
            close_pending: false,
            prepared: None,
            plan_ready: false,
            analysis_generation: 0,
            analysis_cancel: Arc::new(AtomicBool::new(false)),
            analysis_due: None,
            audio: None,
            audio_next: None,
            play_span: None,
            video_running: false,
            last_sync: Instant::now(),
        }
    }
    fn ui(&self) -> &Ui {
        self.ui.as_ref().unwrap()
    }
    unsafe fn init(&mut self, hwnd: HWND) {
        let mut u = Ui::new(hwnd, self.settings.dark);
        for (id, label) in [
            (NEW_PROJECT, "\u{e710}|New project"),
            (IMPORT, "\u{e710}|Import"),
            (THEME, "◐"),
            (PLAY, "\u{e768}|Play"),
            (SPLIT, "\u{e8c6}|Split"),
            (DELETE, "\u{e74d}|Delete"),
            (UNDO, "\u{e7a7}|Undo"),
            (LEFT, "\u{e76b}|Left"),
            (RIGHT, "\u{e76c}|Right"),
            (ZOOM_IN, "+"),
            (ZOOM_OUT, "−"),
            (FIT, "Fit"),
            (EXPORT, "!\u{e898}|Export"),
            (CANCEL, "Cancel"),
            (OPEN, "\u{e8b7}|Open folder"),
        ] {
            u.button(id, label);
        }
        u.add(SCROLL, "SCROLLBAR", "", SBS_HORZ as u32);
        self.ui = Some(u);
        self.video = CreateWindowExW(
            0,
            w("STATIC").as_ptr(),
            w("").as_ptr(),
            WS_CHILD | SS_BLACKRECT,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        self.layout();
        self.enabled();
        DragAcceptFiles(hwnd, 1);
        SetTimer(hwnd, 1, 15, None);
        if let Err(e) = &self.tools {
            self.status = e.clone();
        }
        if std::env::args_os()
            .nth(1)
            .is_some_and(|a| a.to_string_lossy().starts_with("--"))
        {
            return;
        }
        let paths = std::env::args_os()
            .skip(1)
            .filter(|p| p.to_str() != Some("--smoke-test"))
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            self.import(paths);
        }
    }

    unsafe fn geometry(&self) -> (i32, i32, i32, i32) {
        let (w, h) = self.ui().size();
        (w, h, h - 308, h - 256)
    }
    unsafe fn content_width(&self) -> f64 {
        (self.ui().size().0 - 48).max(1) as f64
    }
    unsafe fn layout(&self) {
        let u = self.ui();
        let (w, h, row, _) = self.geometry();
        u.place(NEW_PROJECT, w - 522, 22, 150, 38);
        u.place(IMPORT, w - 360, 22, 126, 38);
        u.place(EXPORT, w - 222, 22, 138, 38);
        u.place(THEME, w - 72, 22, 48, 38);
        MoveWindow(
            self.video,
            u.s(24),
            u.s(78),
            u.s(w - 48),
            u.s((h - 402).max(1)),
            1,
        );
        u.place(PLAY, 24, row, 105, 36);
        u.place(SPLIT, w - 656, row, 100, 36);
        u.place(DELETE, w - 548, row, 108, 36);
        u.place(UNDO, w - 432, row, 101, 36);
        u.place(LEFT, w - 323, row, 88, 36);
        u.place(RIGHT, w - 227, row, 97, 36);
        u.place(ZOOM_OUT, w - 174, h - 60, 40, 36);
        u.place(ZOOM_IN, w - 128, h - 60, 40, 36);
        u.place(FIT, w - 82, h - 60, 58, 36);
        u.place(SCROLL, 24, h - 117, w - 48, 14);
        u.place(CANCEL, w - 322, h - 60, 112, 36);
        u.place(OPEN, w - 376, h - 60, 166, 36);
        if let Some(player) = &self.preview {
            player.resize(u.s(w - 48), u.s((h - 402).max(1)));
        }
        self.update_scroll();
    }

    unsafe fn enabled(&self) {
        let ready = !self.timeline.clips.is_empty();
        for id in [
            NEW_PROJECT,
            IMPORT,
            SPLIT,
            DELETE,
            UNDO,
            LEFT,
            RIGHT,
            EXPORT,
        ] {
            self.ui().enable(
                id,
                !self.busy && (id == IMPORT || id == NEW_PROJECT || ready),
            );
        }
        self.ui()
            .enable(UNDO, !self.busy && !self.history.is_empty());
        self.ui()
            .enable(DELETE, !self.busy && self.selected.is_some());
        self.ui().enable(PLAY, ready && !self.busy);
        self.ui().show(CANCEL, self.busy);
        self.ui().show(OPEN, !self.busy && self.output.is_some());
        self.ui().refresh();
    }
    fn remember(&mut self) {
        self.history.push(self.timeline.clone());
        if self.history.len() > 64 {
            self.history.remove(0);
        }
        self.output = None;
    }
    unsafe fn edited(&mut self) {
        self.playhead = self.playhead.clamp(0., self.timeline.duration());
        if self.timeline.clips.is_empty() {
            self.selected = None;
            self.preview_index = None;
            self.pause();
            ShowWindow(self.video, SW_HIDE);
        } else {
            self.selected = self.selected.map(|i| i.min(self.timeline.clips.len() - 1));
            self.seek(true);
        }
        self.invalidate_plan();
        self.progress = 0.;
        self.status = format!(
            "{} {} · {}",
            self.timeline.clips.len(),
            if self.timeline.clips.len() == 1 {
                "clip"
            } else {
                "clips"
            },
            human_time(self.timeline.duration())
        );
        self.update_scroll();
        self.enabled();
    }

    fn export_state(&self) -> export_ui::ExportState {
        let elapsed = self
            .export_started
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(self.export_elapsed);
        export_ui::ExportState {
            file_name: self.export_name.clone().unwrap_or_else(|| {
                let stem = if self.timeline.clips.len() > 1 {
                    "Timeline".into()
                } else {
                    self.timeline
                        .clips
                        .first()
                        .and_then(|c| c.media.path.file_stem())
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Video".into())
                };
                format!("{stem}-edited")
            }),
            elapsed,
            remaining: if self.export_started.is_some() {
                self.export_estimate.remaining(elapsed)
            } else {
                None
            },
            before_duration: self.timeline.duration(),
            before_size: self.timeline.bytes(),
            after_duration: if !self.silence {
                Some(self.timeline.duration())
            } else if self.plan_ready {
                self.prepared.as_ref().map(|p| p.duration)
            } else {
                None
            },
            output: self.output.clone(),
            busy: self.busy,
            progress: self.progress,
            playing: self.playing,
            status: self.status.clone(),
        }
    }
    unsafe fn sync_export_window(&self) {
        if !self.dialog.is_null() && IsWindow(self.dialog) != 0 {
            PostMessageW(
                self.dialog,
                export_ui::EXPORT_UPDATE,
                Box::into_raw(Box::new(self.export_state())) as usize,
                0,
            );
        }
    }
    unsafe fn dialog_owner(&self) -> HWND {
        if !self.dialog.is_null() && IsWindow(self.dialog) != 0 {
            self.dialog
        } else {
            self.ui().hwnd
        }
    }
    unsafe fn pause(&mut self) {
        self.playing = false;
        self.audio = None;
        self.audio_next = None;
        self.video_running = false;
        if let Some(p) = &self.preview {
            p.pause();
        }
        self.ui().set_text(PLAY, "\u{e768}|Play");
        self.sync_export_window();
    }
    fn span(&self, index: usize) -> Option<PreviewSpan> {
        if let Some(plan) = &self.prepared {
            plan.spans.get(index).cloned()
        } else {
            self.timeline.clips.get(index).map(|c| PreviewSpan {
                clip: index,
                timeline_start: self.timeline.offset(index),
                media: c.media.clone(),
                start: c.start,
                end: c.end,
            })
        }
    }
    fn locate_span(&self, t: f64) -> Option<(usize, f64)> {
        if let Some(plan) = &self.prepared {
            plan.locate(t)
        } else {
            self.timeline.locate(t)
        }
    }
    unsafe fn queue_audio(&self, index: usize, start: f64) -> Option<live_audio::LiveAudio> {
        let s = self.span(index)?;
        if s.media.audio_count == 0 {
            return None;
        }
        Some(live_audio::LiveAudio::prepare(
            self.tools.as_ref().ok()?.clone(),
            s.media,
            start,
            s.end,
            self.settings.level,
        ))
    }
    unsafe fn start_span(
        &mut self,
        index: usize,
        position: f64,
        prefetched: Option<live_audio::LiveAudio>,
    ) {
        let Some(span) = self.span(index) else {
            return;
        };
        self.audio = prefetched.or_else(|| self.queue_audio(index, position));
        self.audio_next = None;
        self.play_span = Some(index);
        self.video_running = false;
        if self.preview.is_none() {
            self.preview = preview::Preview::new(self.video).ok();
        }
        let (w, h, _, _) = self.geometry();
        if let Some(p) = &mut self.preview {
            p.seek(&span.media.path, position, false);
            let dpi = self.ui.as_ref().unwrap().dpi;
            p.resize(
                ((w - 48) as f64 * dpi) as i32,
                ((h - 402).max(1) as f64 * dpi) as i32,
            );
            ShowWindow(self.video, SW_SHOW);
        }
        self.preview_index = Some(span.clip);
        if let Some(next) = self.span(index + 1) {
            self.audio_next = self.queue_audio(index + 1, next.start);
        }
    }
    unsafe fn seek(&mut self, force: bool) {
        if !force && self.last_seek.elapsed() < Duration::from_millis(90) {
            return;
        }
        self.last_seek = Instant::now();
        let located = if self.playing {
            self.locate_span(self.playhead)
        } else {
            self.timeline.locate(self.playhead)
        };
        let Some((i, source)) = located else {
            return;
        };
        if self.playing {
            if let Some(span) = self.span(i) {
                self.playhead = span.timeline_start + (source - span.start);
            }
            self.start_span(i, source, None);
        } else {
            let Some(c) = self.timeline.clips.get(i) else {
                return;
            };
            let path = c.media.path.clone();
            if self.preview.is_none() {
                self.preview = preview::Preview::new(self.video).ok();
            }
            let (w, h, _, _) = self.geometry();
            let dpi = self.ui().dpi;
            if let Some(p) = &mut self.preview {
                p.seek(&path, source, false);
                p.resize(
                    ((w - 48) as f64 * dpi) as i32,
                    ((h - 402).max(1) as f64 * dpi) as i32,
                );
                ShowWindow(self.video, SW_SHOW);
            }
            self.preview_index = Some(i);
        }
    }
    unsafe fn play(&mut self) {
        if self.busy {
            return;
        }
        if self.playing {
            self.pause();
        } else if !self.timeline.clips.is_empty() {
            if self.prepared.as_ref().is_some_and(|p| p.spans.is_empty()) {
                self.status = "No content remains with these silence settings".into();
                self.ui().refresh();
                return;
            }
            if self.locate_span(self.playhead).is_none() {
                self.playhead = 0.;
            }
            self.playing = true;
            self.ui().set_text(PLAY, "\u{e769}|Pause");
            self.seek(true);
        }
        self.ui().refresh();
    }
    unsafe fn invalidate_plan(&mut self) {
        self.analysis_cancel.store(true, Ordering::Release);
        self.analysis_generation += 1;
        self.plan_ready = false;
        if !self.playing {
            self.prepared = None;
        }
        self.analysis_due = Some(Instant::now() + Duration::from_millis(240));
        self.ui().refresh();
    }
    unsafe fn prepare_plan(&mut self) {
        self.analysis_due = None;
        let generation = self.analysis_generation;
        if !self.silence || self.timeline.clips.is_empty() {
            self.prepared = Some(PreviewPlan::uncut(&self.timeline));
            self.plan_ready = true;
            self.ui().refresh();
            return;
        }
        let Ok(tools) = &self.tools else {
            return;
        };
        let tools = tools.clone();
        let timeline = self.timeline.clone();
        let settings = self.settings.clone();
        let tx = self.tx.clone();
        let hwnd = self.ui().hwnd as usize;
        let cancel = Arc::new(AtomicBool::new(false));
        self.analysis_cancel = cancel.clone();
        self.status = "Preparing silence preview…".into();
        std::thread::spawn(move || {
            let mut last = Instant::now() - Duration::from_secs(1);
            let result =
                quietcut::plan::prepare(&tools, &timeline, &settings, true, &cancel, |_, msg| {
                    if last.elapsed() > Duration::from_millis(120) {
                        last = Instant::now();
                        let _ = tx.send(Event::Preparing(generation, msg.into()));
                        PostMessageW(hwnd as HWND, EVENT, 0, 0);
                    }
                });
            let _ = tx.send(Event::Prepared(generation, result));
            PostMessageW(hwnd as HWND, EVENT, 0, 0);
        });
    }
    unsafe fn apply_options(&mut self, mut req: export_ui::ExportRequest) {
        req.settings.exact_silence(req.silence);
        let changed = self.silence != req.silence || !self.settings.detection_eq(&req.settings);
        let level = self.settings.level != req.settings.level;
        if changed || level || self.settings.quality != req.settings.quality {
            self.output = None;
        }
        self.settings = req.settings;
        self.silence = req.silence;
        save_settings(&self.settings);
        if changed {
            self.invalidate_plan();
            if !self.silence {
                self.prepared = Some(PreviewPlan::uncut(&self.timeline));
                self.plan_ready = true;
                self.analysis_due = None;
                if self.playing {
                    self.seek(true);
                }
            }
        }
        if level && self.playing {
            self.seek(true);
        }
        self.ui().refresh();
        self.sync_export_window();
    }
    unsafe fn advance(&mut self) {
        let Some(index) = self.play_span else {
            return;
        };
        if let Some(next) = self.span(index + 1) {
            let ready = self.audio_next.take();
            self.playhead = next.timeline_start;
            self.start_span(index + 1, next.start, ready);
        } else {
            self.playhead = self.timeline.duration();
            self.pause();
        }
    }
    unsafe fn tick(&mut self) {
        if self.export_started.is_some()
            && self.export_refreshed.elapsed() >= Duration::from_millis(250)
        {
            self.export_refreshed = Instant::now();
            self.sync_export_window();
        }
        if self.analysis_due.is_some_and(|t| Instant::now() >= t) && !self.busy {
            self.prepare_plan();
        }
        if let Some(p) = &mut self.preview {
            p.tick();
            if p.failed() {
                ShowWindow(self.video, SW_HIDE);
            }
        }
        if !self.playing {
            return;
        }
        let Some(index) = self.play_span else {
            return;
        };
        let Some(span) = self.span(index) else {
            self.pause();
            return;
        };
        if let Some(audio) = &self.audio {
            if let Some(error) = audio.error() {
                self.pause();
                self.status = error;
                self.ui().refresh();
                return;
            }
            let video_ready = self
                .preview
                .as_ref()
                .is_none_or(|p| p.ready() || p.failed());
            if audio.ready() && video_ready && !self.video_running {
                audio.start();
                if let Some(p) = &self.preview {
                    p.set_playing(true);
                }
                self.video_running = true;
            }
            if audio.running() {
                let position = audio.position();
                self.playhead = span.timeline_start + (position - span.start);
                if self.last_sync.elapsed() > Duration::from_millis(400) {
                    if let Some(p) = &mut self.preview {
                        if p.position().is_some_and(|t| (t - position).abs() > 0.12) {
                            p.seek(&span.media.path, position, true);
                        }
                    }
                    self.last_sync = Instant::now();
                }
            }
            if audio.finished() {
                self.advance();
            }
        } else {
            if !self.video_running && self.preview.as_ref().is_some_and(|p| p.ready()) {
                if let Some(p) = &self.preview {
                    p.set_playing(true);
                }
                self.video_running = true;
            }
            if let Some(position) = self.preview.as_ref().and_then(|p| p.position()) {
                self.playhead = span.timeline_start + (position - span.start).max(0.);
                if position >= span.end - 0.005 {
                    self.advance();
                }
            }
        }
        let width = self.content_width();
        if self.playhead < self.scroll || self.playhead > self.scroll + width / self.pps {
            self.scroll = self.playhead;
            self.update_scroll();
        }
        let (_, h, row, _) = self.geometry();
        let r = RECT {
            left: 0,
            top: self.ui().s(row),
            right: self.ui().s(self.ui().size().0),
            bottom: self.ui().s(h - 113),
        };
        InvalidateRect(self.ui().hwnd, &r, 0);
    }

    unsafe fn import_dialog(&self) -> Vec<PathBuf> {
        let mut b = vec![0u16; 65536];
        let f = w(
            "Video files\0*.mp4;*.mkv;*.mov;*.avi;*.webm;*.m4v;*.wmv;*.ts;*.mts;*.m2ts\0All files\0*.*\0",
        );
        let mut o: OPENFILENAMEW = zeroed();
        o.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        o.hwndOwner = self.ui().hwnd;
        o.lpstrFilter = f.as_ptr();
        o.lpstrFile = b.as_mut_ptr();
        o.nMaxFile = b.len() as u32;
        o.Flags = OFN_EXPLORER
            | OFN_ALLOWMULTISELECT
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut o) == 0 {
            return vec![];
        }
        let parts = b
            .split(|x| *x == 0)
            .take_while(|p| !p.is_empty())
            .map(OsString::from_wide)
            .collect::<Vec<_>>();
        if parts.len() == 1 {
            vec![PathBuf::from(&parts[0])]
        } else {
            parts
                .iter()
                .skip(1)
                .map(|p| PathBuf::from(&parts[0]).join(p))
                .collect()
        }
    }
    unsafe fn import(&mut self, paths: Vec<PathBuf>) {
        if self.busy || paths.is_empty() {
            return;
        }
        let tools = match &self.tools {
            Ok(t) => t.clone(),
            Err(e) => {
                let e = e.clone();
                self.error(&e);
                return;
            }
        };
        self.pause();
        self.busy = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status = "Importing…".into();
        self.enabled();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let hwnd = self.ui().hwnd as usize;
        std::thread::spawn(move || {
            let mut media = vec![];
            let mut errors = vec![];
            for path in paths {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                match probe(&tools, &path, &cancel) {
                    Ok(m) => media.push(m),
                    Err(e) => errors.push(e),
                }
            }
            let _ = tx.send(Event::Imported(media, errors));
            PostMessageW(hwnd as HWND, EVENT, 0, 0);
        });
    }
    unsafe fn error(&mut self, e: &str) {
        self.status = if e == "Cancelled" {
            "Cancelled · no output saved".into()
        } else {
            "Could not complete".into()
        };
        if e != "Cancelled" {
            MessageBoxW(
                self.dialog_owner(),
                w(e).as_ptr(),
                w("QuietCut").as_ptr(),
                MB_OK | MB_ICONWARNING,
            );
        }
        self.ui().refresh();
    }
    unsafe fn save_dialog(&self, settings: &Settings, file_name: &str) -> Result<Option<PathBuf>> {
        let first = &self
            .timeline
            .clips
            .first()
            .ok_or("Import a video first.")?
            .media
            .path;
        let ext = settings.extension();
        let folder = settings
            .output_dir
            .as_deref()
            .unwrap_or_else(|| first.parent().unwrap());
        let proposed = proposed_output(folder, file_name, ext)?;
        if !settings.ask_save {
            return Ok(Some(proposed));
        }
        let val = proposed.as_os_str().encode_wide().collect::<Vec<_>>();
        let mut b = vec![0u16; 32768];
        if val.len() >= b.len() {
            return Err("The output path is too long.".into());
        }
        b[..val.len()].copy_from_slice(&val);
        let filter = w(&format!("{} video\0*.{ext}\0", ext.to_uppercase()));
        let def = w(ext);
        let mut o: OPENFILENAMEW = zeroed();
        o.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        o.hwndOwner = self.dialog_owner();
        o.lpstrFilter = filter.as_ptr();
        o.lpstrDefExt = def.as_ptr();
        o.lpstrFile = b.as_mut_ptr();
        o.nMaxFile = b.len() as u32;
        o.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetSaveFileNameW(&mut o) != 0 {
            Ok(Some(PathBuf::from(OsString::from_wide(
                &b[..b.iter().position(|x| *x == 0).unwrap_or(0)],
            ))))
        } else {
            Ok(None)
        }
    }
    unsafe fn export(&mut self, request: export_ui::ExportRequest) {
        if self.busy {
            return;
        }
        let tools = match &self.tools {
            Ok(t) => t.clone(),
            Err(e) => {
                let e = e.clone();
                self.error(&e);
                return;
            }
        };
        let name = request
            .file_name
            .clone()
            .unwrap_or_else(|| self.export_state().file_name);
        let path = match self.save_dialog(&request.settings, &name) {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.sync_export_window();
                return;
            }
            Err(error) => {
                self.error(&error);
                self.sync_export_window();
                return;
            }
        };
        self.pause();
        self.settings = request.settings.clone();
        self.silence = request.silence;
        save_settings(&self.settings);
        self.busy = true;
        self.progress = 0.;
        self.output = None;
        self.export_name = Some(name);
        self.export_started = Some(Instant::now());
        self.export_elapsed = 0.;
        self.export_estimate = ProgressEstimate::default();
        self.status = "Preparing export…".into();
        self.sync_export_window();
        self.analysis_cancel.store(true, Ordering::Release);
        self.analysis_generation += 1;
        self.analysis_due = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.enabled();
        let timeline = self.timeline.clone();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let hwnd = self.ui().hwnd as usize;
        std::thread::spawn(move || {
            let mut last = Instant::now() - Duration::from_secs(1);
            let result = export_timeline(
                &tools,
                &timeline,
                &request.settings,
                request.silence,
                &path,
                &cancel,
                |p, s| {
                    if last.elapsed() > Duration::from_millis(80) || p >= 1. {
                        last = Instant::now();
                        let _ = tx.send(Event::Progress(p, s.into()));
                        PostMessageW(hwnd as HWND, EVENT, 0, 0);
                    }
                },
            );
            let _ = tx.send(Event::Exported(result));
            PostMessageW(hwnd as HWND, EVENT, 0, 0);
        });
    }
    unsafe fn events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Imported(media, errors) => {
                    self.busy = false;
                    if !media.is_empty() {
                        self.remember();
                        for m in media {
                            self.timeline.add(m);
                        }
                        self.selected = Some(self.timeline.clips.len() - 1);
                        self.fit();
                        self.edited();
                    } else {
                        self.status = "Import or drop videos".into();
                    }
                    if !errors.is_empty() {
                        self.error(&errors.join("\n\n"));
                    }
                }
                Event::Progress(p, s) => {
                    if let Some(started) = self.export_started {
                        self.export_estimate
                            .observe(started.elapsed().as_secs_f64(), p);
                    }
                    self.progress = p;
                    self.status = s;
                }
                Event::Preparing(generation, msg) => {
                    if generation == self.analysis_generation && !self.busy {
                        self.status = msg;
                    }
                }
                Event::Prepared(generation, result) => {
                    if generation == self.analysis_generation {
                        match result {
                            Ok(plan) => {
                                let empty = plan.spans.is_empty();
                                self.prepared = Some(plan);
                                self.plan_ready = true;
                                if empty {
                                    self.pause();
                                } else if self.playing {
                                    self.seek(true);
                                }
                                if !self.busy {
                                    self.status = if empty {
                                        "No content remains with these silence settings"
                                    } else {
                                        "Preview ready"
                                    }
                                    .into();
                                }
                            }
                            Err(e) => {
                                if e != "Cancelled" && !self.busy {
                                    self.status = e;
                                }
                            }
                        }
                    }
                }
                Event::Exported(result) => {
                    self.busy = false;
                    self.export_elapsed = self
                        .export_started
                        .take()
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.);
                    self.progress = 0.;
                    match result {
                        Ok(o) => {
                            self.status = format!(
                                "Saved {}",
                                o.path.file_name().unwrap_or_default().to_string_lossy()
                            );
                            self.output = Some(o);
                            self.progress = 1.;
                            let sounded =
                                windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(
                                    MB_ICONASTERISK,
                                );
                            if std::env::args_os()
                                .nth(1)
                                .is_some_and(|a| a == "--export-window-test")
                            {
                                println!("Completion sound requested: {}", sounded != 0);
                            }
                        }
                        Err(e) => self.error(&e),
                    }
                }
            }
        }
        self.enabled();
        self.sync_export_window();
        if self.close_pending && !self.busy {
            DestroyWindow(self.ui().hwnd);
        }
    }
    unsafe fn fit(&mut self) {
        self.pps = (self.content_width() / self.timeline.duration().max(1.)).clamp(0.02, 250.);
        self.scroll = 0.;
        self.update_scroll();
    }
    unsafe fn update_scroll(&self) {
        let u = self.ui();
        let visible = self.content_width() / self.pps;
        let duration = self.timeline.duration();
        let s = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: (duration * 100.) as i32,
            nPage: (visible * 100.) as u32,
            nPos: (self.scroll * 100.) as i32,
            nTrackPos: 0,
        };
        SetScrollInfo(u.get(SCROLL), SB_CTL, &s, 1);
        u.show(SCROLL, duration > visible + 0.01);
    }
    unsafe fn command(&mut self, id: usize) {
        match id {
            NEW_PROJECT if !self.busy => {
                if confirm_new_project(self.dialog_owner()) {
                    self.reset_project();
                }
            }
            THEME => {
                self.settings.dark = !self.settings.dark;
                let dark = self.settings.dark;
                self.ui.as_mut().unwrap().theme(dark);
                save_settings(&self.settings);
            }
            IMPORT => {
                let paths = self.import_dialog();
                self.import(paths);
            }
            PLAY => self.play(),
            SPLIT if !self.busy => {
                self.pause();
                self.remember();
                if self.timeline.split(self.playhead) {
                    self.selected = self.timeline.locate(self.playhead).map(|(i, _)| i);
                    self.edited();
                } else {
                    self.history.pop();
                }
            }
            DELETE if !self.busy => {
                if let Some(i) = self.selected {
                    self.pause();
                    self.remember();
                    let offset = self.timeline.offset(i);
                    self.timeline.remove(i);
                    self.playhead = offset;
                    self.edited();
                }
            }
            UNDO if !self.busy => {
                if let Some(t) = self.history.pop() {
                    self.pause();
                    self.timeline = t;
                    self.output = None;
                    self.edited();
                }
            }
            LEFT | RIGHT if !self.busy => {
                if let Some(i) = self.selected {
                    let to = if id == LEFT {
                        i.saturating_sub(1)
                    } else {
                        (i + 1).min(self.timeline.clips.len() - 1)
                    };
                    if to != i {
                        self.pause();
                        self.remember();
                        self.timeline.reorder(i, to);
                        self.selected = Some(to);
                        self.playhead = self.timeline.offset(to);
                        self.edited();
                    }
                }
            }
            ZOOM_IN | ZOOM_OUT => {
                let old = self.pps;
                self.pps =
                    (self.pps * if id == ZOOM_IN { 1.5 } else { 1. / 1.5 }).clamp(0.02, 500.);
                self.scroll = (self.playhead - (self.playhead - self.scroll) * old / self.pps)
                    .max(0.)
                    .min((self.timeline.duration() - self.content_width() / self.pps).max(0.));
                self.update_scroll();
                self.ui().refresh();
            }
            FIT => {
                self.fit();
                self.ui().refresh();
            }
            EXPORT if !self.busy && !self.timeline.clips.is_empty() => {
                self.pause();
                self.dialog = export_ui::ExportUi::open(
                    self.ui().hwnd,
                    self.settings.clone(),
                    self.silence,
                    self.export_state(),
                );
            }
            CANCEL => {
                self.cancel.store(true, Ordering::Relaxed);
                self.status = "Cancelling…".into();
                self.ui().refresh();
            }
            OPEN => {
                if let Some(o) = &self.output {
                    ShellExecuteW(
                        self.ui().hwnd,
                        w("open").as_ptr(),
                        o.path
                            .parent()
                            .unwrap_or(&o.path)
                            .as_os_str()
                            .encode_wide()
                            .chain(Some(0))
                            .collect::<Vec<_>>()
                            .as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                }
            }
            _ => {}
        }
    }
    unsafe fn reset_project(&mut self) {
        self.pause();
        self.analysis_cancel.store(true, Ordering::Release);
        self.analysis_generation += 1;
        self.analysis_due = None;
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.timeline = Timeline::default();
        self.history.clear();
        self.selected = None;
        self.playhead = 0.;
        self.scroll = 0.;
        self.pps = 1.;
        self.drag = Drag::None;
        self.preview = None;
        self.preview_index = None;
        self.play_span = None;
        self.prepared = None;
        self.plan_ready = false;
        self.output = None;
        self.export_name = None;
        self.export_started = None;
        self.export_estimate = ProgressEstimate::default();
        self.export_elapsed = 0.;
        self.progress = 0.;
        self.status = "Import or drop videos".into();
        ShowWindow(self.video, SW_HIDE);
        let dialog = std::mem::replace(&mut self.dialog, null_mut());
        if !dialog.is_null() && IsWindow(dialog) != 0 {
            DestroyWindow(dialog);
        }
        self.update_scroll();
        self.enabled();
        SetFocus(self.ui().get(IMPORT));
    }
    fn time_at(&self, x: i32) -> f64 {
        ((x - 24) as f64 / self.pps + self.scroll).clamp(0., self.timeline.duration())
    }
    fn clip_at(&self, x: i32) -> Option<usize> {
        self.timeline.locate(self.time_at(x)).map(|(i, _)| i)
    }
    unsafe fn down(&mut self, x: i32, y: i32) {
        let (w, _, _, top) = self.geometry();
        if self.busy
            || self.timeline.clips.is_empty()
            || !(24..=w - 24).contains(&x)
            || !(top..=top + 131).contains(&y)
        {
            return;
        }
        self.pause();
        SetFocus(self.ui().hwnd);
        SetCapture(self.ui().hwnd);
        if y < top + 29 {
            self.drag = Drag::Scrub;
            self.playhead = self.time_at(x);
            self.seek(false);
        } else if let Some(i) = self.clip_at(x) {
            let a = 24. + (self.timeline.offset(i) - self.scroll) * self.pps;
            let b = a + self.timeline.clips[i].duration() * self.pps;
            self.selected = Some(i);
            if (x as f64 - a).abs() < 9. || (x as f64 - b).abs() < 9. {
                self.drag = Drag::Trim {
                    index: i,
                    left: (x as f64 - a).abs() < (x as f64 - b).abs(),
                    x,
                    original: self.timeline.clone(),
                };
            } else {
                self.playhead = self.time_at(x);
                self.drag = Drag::Clip {
                    index: i,
                    x,
                    target: i,
                    moved: false,
                };
                self.seek(true);
            }
            self.enabled();
        }
        self.ui().refresh();
    }
    unsafe fn motion(&mut self, x: i32) {
        match self.drag.clone() {
            Drag::Scrub => {
                self.playhead = self.time_at(x);
                self.seek(false);
            }
            Drag::Clip {
                index, x: start, ..
            } => {
                let target = self.clip_at(x).unwrap_or(index);
                self.drag = Drag::Clip {
                    index,
                    x: start,
                    target,
                    moved: (x - start).abs() > 8,
                };
            }
            Drag::Trim {
                index,
                left,
                x: start,
                original,
            } => {
                let old = &original.clips[index];
                let frame = 1. / old.media.fps;
                let delta = (x - start) as f64 / self.pps;
                let snap = |v: f64| (v / frame).round() * frame;
                let clip = &mut self.timeline.clips[index];
                if left {
                    clip.start = snap(old.start + delta).clamp(0., old.end - frame);
                } else {
                    clip.end = snap(old.end + delta).clamp(old.start + frame, old.media.duration);
                }
                self.output = None;
                self.update_scroll();
            }
            Drag::None => return,
        }
        self.ui().refresh();
    }
    unsafe fn up(&mut self) {
        ReleaseCapture();
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::Clip {
                index,
                target,
                moved: true,
                ..
            } if index != target => {
                self.remember();
                self.timeline.reorder(index, target);
                self.selected = Some(target);
                self.playhead = self.timeline.offset(target);
                self.edited();
            }
            Drag::Trim { original, .. } => {
                self.history.push(original);
                self.edited();
            }
            Drag::Scrub => self.seek(true),
            _ => {}
        }
        self.ui().refresh();
    }

    unsafe fn paint(&self, dc: HDC, width: i32, height: i32) {
        let u = self.ui();
        let p = u.colors;
        let (w, h, row, top) = self.geometry();
        let full = w - 48;
        fill(
            dc,
            RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
            p.bg,
        );
        u.label(dc, "\u{e714}", 24, 20, 30, 39, p.accent, 3);
        u.label(dc, "QuietCut 1.8", 62, 20, 260, 39, p.fg, 1);
        u.rect(dc, 24, 68, full, 1, 0, p.line, p.line);
        u.rect(dc, 24, 78, full, h - 402, 10, p.panel, p.line);
        if self.timeline.clips.is_empty() {
            u.label(
                dc,
                "\u{e8b9}",
                w / 2 - 20,
                78 + (h - 402) / 2 - 45,
                50,
                40,
                p.accent,
                3,
            );
            u.label(
                dc,
                "Drop videos here",
                w / 2 - 95,
                78 + (h - 402) / 2,
                250,
                30,
                p.fg,
                2,
            );
        } else if self.preview.is_none() || self.preview.as_ref().is_some_and(|p| p.failed()) {
            u.label(
                dc,
                "Video preview unavailable",
                w / 2 - 145,
                78 + (h - 402) / 2,
                350,
                30,
                p.muted,
                0,
            );
        }
        u.label(
            dc,
            &format!(
                "{} / {}",
                human_time(self.playhead),
                human_time(self.timeline.duration())
            ),
            146,
            row,
            190,
            36,
            p.fg,
            0,
        );
        u.rect(dc, 24, top, full, 132, 8, p.panel, p.line);
        let saved = SaveDC(dc);
        IntersectClipRect(dc, u.s(25), u.s(top + 1), u.s(w - 25), u.s(top + 131));
        let interval = [
            0.1, 0.5, 1., 2., 5., 10., 30., 60., 120., 300., 600., 1800., 3600.,
        ]
        .into_iter()
        .find(|v| v * self.pps > 70.)
        .unwrap_or(3600.);
        let mut t = (self.scroll / interval).floor() * interval;
        while !self.timeline.clips.is_empty() && t < self.scroll + full as f64 / self.pps {
            let x = (24. + (t - self.scroll) * self.pps) as i32;
            u.label(dc, &human_time(t), x + 5, top + 3, 110, 22, p.muted, 0);
            u.rect(dc, x, top + 25, 1, 105, 0, p.line, p.line);
            t += interval;
        }
        let mut offset = 0.;
        for (i, c) in self.timeline.clips.iter().enumerate() {
            let x = (24. + (offset - self.scroll) * self.pps) as i32;
            let width = (c.duration() * self.pps) as i32;
            let clip_offset = offset;
            offset += c.duration();
            if x + width < 24 || x > w - 24 {
                continue;
            }
            let selected = self.selected == Some(i);
            let color = if selected {
                if u.dark {
                    rgb(47, 47, 56)
                } else {
                    rgb(221, 221, 231)
                }
            } else {
                p.field
            };
            u.rect(
                dc,
                x + 1,
                top + 36,
                (width - 2).max(3),
                81,
                6,
                color,
                if selected { p.accent } else { p.line },
            );
            if self.silence {
                if let Some(plan) = &self.prepared {
                    let mut cursor = clip_offset;
                    let finish = clip_offset + c.duration();
                    let begin = plan.spans.partition_point(|s| s.clip < i);
                    let end = plan.spans.partition_point(|s| s.clip <= i);
                    for span in &plan.spans[begin..end] {
                        if span.timeline_start > cursor {
                            let a = (24. + (cursor - self.scroll) * self.pps) as i32;
                            let z = ((span.timeline_start - cursor) * self.pps) as i32;
                            u.rect(
                                dc,
                                a,
                                top + 108,
                                z,
                                7,
                                1,
                                if u.dark {
                                    rgb(116, 65, 69)
                                } else {
                                    rgb(218, 153, 153)
                                },
                                p.line,
                            );
                        }
                        cursor = span.timeline_start + span.len();
                    }
                    if cursor < finish {
                        let a = (24. + (cursor - self.scroll) * self.pps) as i32;
                        u.rect(
                            dc,
                            a,
                            top + 108,
                            ((finish - cursor) * self.pps) as i32,
                            7,
                            1,
                            if u.dark {
                                rgb(116, 65, 69)
                            } else {
                                rgb(218, 153, 153)
                            },
                            p.line,
                        );
                    }
                }
            }
            if selected && width > 20 {
                for xx in [x + 5, x + width - 7] {
                    u.rect(dc, xx, top + 58, 3, 35, 1, p.accent, p.accent);
                }
            }
            if width > 40 {
                u.label(dc, "\u{e714}", x + 14, top + 47, 24, 24, p.accent, 3);
                u.label(
                    dc,
                    &c.media
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    x + 42,
                    top + 47,
                    (width - 55).max(5),
                    24,
                    p.fg,
                    0,
                );
                u.label(
                    dc,
                    &human_time(c.duration()),
                    x + 14,
                    top + 83,
                    (width - 28).max(5),
                    23,
                    p.muted,
                    0,
                );
            }
        }
        if let Drag::Clip {
            target,
            moved: true,
            ..
        } = &self.drag
        {
            let x = (24. + (self.timeline.offset(*target) - self.scroll) * self.pps) as i32;
            u.rect(dc, x, top + 34, 3, 85, 0, p.accent, p.accent);
        }
        if !self.timeline.clips.is_empty() {
            let x = (24. + (self.playhead - self.scroll) * self.pps) as i32;
            u.rect(dc, x - 1, top + 3, 2, 123, 0, p.accent, p.accent);
            u.rect(dc, x - 5, top, 10, 6, 2, p.accent, p.accent);
        }
        RestoreDC(dc, saved);
        u.label(dc, &self.status, 24, h - 60, w - 520, 36, p.muted, 0);
        if self.busy {
            u.rect(dc, 24, h - 97, full, 4, 2, p.line, p.line);
            u.rect(
                dc,
                24,
                h - 97,
                (full as f64 * self.progress.clamp(0., 1.)) as i32,
                4,
                2,
                p.accent,
                p.accent,
            );
        }
    }
}
thread_local! {static DISPATCHING:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        DISPATCHING.with(|b| b.set(false));
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(v) = early(hwnd, msg, wp, lp) {
        return v;
    }
    if msg == CLEANUP_DIALOG {
        drop(Box::from_raw(wp as *mut export_ui::ExportUi));
        return 0;
    }
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    // Native default handling dispatches WM_SIZE synchronously during
    // maximize, restore and resize. Forward it before taking the app borrow
    // or dispatch guard, so the nested size event can lay out the controls.
    if !matches!(
        msg,
        WM_CREATE
            | WM_SIZE
            | WM_GETMINMAXINFO
            | WM_DPICHANGED
            | export_ui::PREVIEW_SETTINGS
            | export_ui::PREVIEW_PLAY
            | export_ui::EXPORT_CANCEL
            | WM_COMMAND
            | EVENT
            | WM_TIMER
            | export_ui::EXPORT_SETTINGS
            | WM_PAINT
            | WM_LBUTTONDOWN
            | WM_MOUSEMOVE
            | WM_LBUTTONUP
            | WM_MOUSEWHEEL
            | WM_HSCROLL
            | WM_KEYDOWN
            | WM_DROPFILES
            | WM_CLOSE
    ) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if DISPATCHING.with(|b| b.replace(true)) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let _guard = Guard;
    let a = &mut *ptr;
    match msg {
        WM_CREATE => {
            a.init(hwnd);
            0
        }
        WM_SIZE => {
            if a.ui.is_some() && wp != SIZE_MINIMIZED as usize {
                a.layout();
                a.ui().refresh();
            }
            0
        }
        WM_GETMINMAXINFO => {
            let m = &mut *(lp as *mut MINMAXINFO);
            let dpi = GetDpiForWindow(hwnd) as i32;
            m.ptMinTrackSize = POINT {
                x: 1000 * dpi / 96,
                y: 660 * dpi / 96,
            };
            0
        }
        WM_DPICHANGED => {
            if let Some(ui) = &mut a.ui {
                ui.update_dpi((wp & 0xffff) as f64 / 96.);
            }
            let r = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            a.layout();
            a.ui().refresh();
            0
        }
        export_ui::PREVIEW_SETTINGS => {
            let req = Box::from_raw(wp as *mut export_ui::ExportRequest);
            a.apply_options(*req);
            0
        }
        export_ui::EXPORT_CANCEL => {
            a.command(CANCEL);
            a.sync_export_window();
            0
        }
        export_ui::PREVIEW_PLAY => {
            a.play();
            a.sync_export_window();
            0
        }
        WM_COMMAND => {
            a.command(wp & 0xffff);
            0
        }
        EVENT => {
            a.events();
            0
        }
        WM_TIMER => {
            a.tick();
            0
        }
        export_ui::EXPORT_SETTINGS => {
            let req = Box::from_raw(wp as *mut export_ui::ExportRequest);
            a.export(*req);
            0
        }
        WM_PAINT => {
            paint(hwnd, |dc, w, h| a.paint(dc, w, h));
            0
        }
        WM_LBUTTONDOWN => {
            a.down(
                (lp as i16 as f64 / a.ui().dpi) as i32,
                ((lp >> 16) as i16 as f64 / a.ui().dpi) as i32,
            );
            0
        }
        WM_MOUSEMOVE => {
            if wp & MK_LBUTTON != 0 {
                a.motion((lp as i16 as f64 / a.ui().dpi) as i32);
            }
            0
        }
        WM_LBUTTONUP => {
            a.up();
            0
        }
        WM_MOUSEWHEEL => {
            if wp & MK_SHIFT != 0 {
                a.scroll = (a.scroll - ((wp >> 16) as i16 as f64) / 120. * 120. / a.pps).clamp(
                    0.,
                    (a.timeline.duration() - a.content_width() / a.pps).max(0.),
                );
                a.update_scroll();
                a.ui().refresh();
            } else {
                a.command(if (wp >> 16) as i16 > 0 {
                    ZOOM_IN
                } else {
                    ZOOM_OUT
                });
            }
            0
        }
        WM_HSCROLL => {
            let mut info: SCROLLINFO = zeroed();
            info.cbSize = size_of::<SCROLLINFO>() as u32;
            info.fMask = SIF_ALL;
            GetScrollInfo(a.ui().get(SCROLL), SB_CTL, &mut info);
            match wp & 0xffff {
                0 => a.scroll -= 40. / a.pps,
                1 => a.scroll += 40. / a.pps,
                2 => a.scroll -= 800. / a.pps,
                3 => a.scroll += 800. / a.pps,
                4 | 5 => a.scroll = info.nTrackPos as f64 / 100.,
                _ => {}
            }
            a.scroll = a.scroll.clamp(
                0.,
                (a.timeline.duration() - a.content_width() / a.pps).max(0.),
            );
            a.update_scroll();
            a.ui().refresh();
            0
        }
        WM_KEYDOWN => {
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            match wp as u16 {
                VK_SPACE => a.play(),
                VK_DELETE => a.command(DELETE),
                0x42 if ctrl => a.command(SPLIT),
                0x5a if ctrl => a.command(UNDO),
                0x4f if ctrl => a.command(IMPORT),
                0x4e if ctrl => a.command(NEW_PROJECT),
                VK_LEFT | VK_RIGHT => {
                    a.pause();
                    let frame = a
                        .timeline
                        .locate(a.playhead)
                        .map(|(i, _)| 1. / a.timeline.clips[i].media.fps)
                        .unwrap_or(1. / 30.);
                    a.playhead = (a.playhead + if wp as u16 == VK_LEFT { -frame } else { frame })
                        .clamp(0., a.timeline.duration());
                    a.seek(true);
                    a.ui().refresh();
                }
                VK_HOME => {
                    a.playhead = 0.;
                    a.seek(true);
                    a.ui().refresh();
                }
                VK_END => {
                    a.playhead = a.timeline.duration();
                    a.seek(true);
                    a.ui().refresh();
                }
                _ => {}
            }
            0
        }
        WM_DROPFILES => {
            let h = wp as HDROP;
            let mut paths = vec![];
            for i in 0..DragQueryFileW(h, u32::MAX, null_mut(), 0) {
                let n = DragQueryFileW(h, i, null_mut(), 0);
                let mut b = vec![0u16; n as usize + 1];
                DragQueryFileW(h, i, b.as_mut_ptr(), n + 1);
                paths.push(PathBuf::from(OsString::from_wide(&b[..n as usize])));
            }
            DragFinish(h);
            a.import(paths);
            0
        }
        WM_CLOSE => {
            if a.busy {
                a.close_pending = true;
                a.cancel.store(true, Ordering::Relaxed);
                a.status = "Cancelling…".into();
                a.ui().refresh();
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn verify_native_layout(app: &App) {
    let u = app.ui();
    let mut client: RECT = zeroed();
    GetClientRect(u.hwnd, &mut client);
    let bounds = |hwnd| {
        let mut r: RECT = zeroed();
        GetWindowRect(hwnd, &mut r);
        MapWindowPoints(null_mut(), u.hwnd, (&mut r as *mut RECT).cast(), 2);
        r
    };
    for (id, hwnd) in &u.controls {
        if GetWindowLongW(*hwnd, GWL_STYLE) as u32 & WS_VISIBLE == 0 {
            continue;
        }
        let r = bounds(*hwnd);
        assert!(
            r.left >= 0 && r.top >= 0 && r.right <= client.right && r.bottom <= client.bottom,
            "Control {id} outside client: {},{}..{},{} in {}x{}",
            r.left,
            r.top,
            r.right,
            r.bottom,
            client.right,
            client.bottom
        );
    }
    let export = bounds(u.get(EXPORT));
    assert!(
        (client.right - export.right - u.s(84)).abs() <= 1,
        "Export lost right anchor"
    );
    assert!(
        (export.top - u.s(22)).abs() <= 1,
        "Export lost header anchor"
    );
    let import = bounds(u.get(IMPORT));
    assert!(
        (client.right - import.right - u.s(234)).abs() <= 1,
        "Import lost right anchor"
    );
    let zoom = bounds(u.get(ZOOM_IN));
    assert!(
        zoom.top > bounds(u.get(SCROLL)).bottom,
        "Zoom overlaps timeline"
    );
    assert!(import.right < export.left, "Header actions overlap");
    let video = bounds(app.video);
    let play = bounds(u.get(PLAY));
    let scroll = bounds(u.get(SCROLL));
    assert!(play.top > video.bottom, "Transport overlaps video");
    assert!(scroll.top > play.bottom, "Timeline overlaps transport");
    println!(
        "Layout verified: {}x{} at {:.0}% DPI",
        client.right,
        client.bottom,
        u.dpi * 100.
    );
}
unsafe fn confirm_new_project(owner: HWND) -> bool {
    MessageBoxW(
        owner,
        w("Remove all videos and start a new project?").as_ptr(),
        w("New project").as_ptr(),
        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
    ) == IDYES
}

// Drive only this test process's own confirmation window; never other apps.
unsafe extern "system" fn find_project_confirmation(hwnd: HWND, parameter: LPARAM) -> i32 {
    let mut title = [0_u16; 64];
    let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
    if String::from_utf16_lossy(&title[..len.max(0) as usize]) == "New project" {
        *(parameter as *mut HWND) = hwnd;
        return 0;
    }
    1
}
unsafe fn project_confirmation_test(app: &mut App, answer: i32) {
    let owner = app.dialog_owner() as usize;
    let owner_thread = GetWindowThreadProcessId(owner as HWND, null_mut());
    let responder = std::thread::spawn(move || unsafe {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let mut dialog = null_mut();
            EnumThreadWindows(
                owner_thread,
                Some(find_project_confirmation),
                (&mut dialog as *mut HWND) as LPARAM,
            );
            if dialog != owner as HWND
                && GetWindowThreadProcessId(dialog, null_mut()) == owner_thread
            {
                let mut title = [0_u16; 64];
                let len = GetWindowTextW(dialog, title.as_mut_ptr(), title.len() as i32);
                if String::from_utf16_lossy(&title[..len.max(0) as usize]) == "New project" {
                    let button = GetDlgItem(dialog, answer);
                    if !button.is_null() {
                        PostMessageW(button, BM_CLICK, 0, 0);
                        return;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        eprintln!("Test confirmation window was not available");
        std::process::exit(3);
    });
    app.command(NEW_PROJECT);
    responder.join().unwrap();
}
fn main() {
    unsafe {
        let arguments: Vec<OsString> = std::env::args_os().collect();
        if arguments.get(1).is_some_and(|a| a == "--audio-smoke-test") {
            let result = (|| -> Result<()> {
                let tools = Tools::find()?;
                let path = arguments.get(2).ok_or("Expected silent test video")?;
                let cancel = Arc::new(AtomicBool::new(false));
                let media = probe(&tools, std::path::Path::new(path), &cancel)?;
                let audio = live_audio::LiveAudio::prepare(tools, Arc::new(media), 0., 1., true);
                let start = Instant::now();
                while !audio.ready()
                    && audio.error().is_none()
                    && start.elapsed() < Duration::from_secs(8)
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if let Some(e) = audio.error() {
                    return Err(e);
                }
                if !audio.ready() {
                    return Err("Audio readiness timeout".into());
                }
                let preparation = start.elapsed().as_millis();
                audio.start();
                while !audio.finished() && start.elapsed() < Duration::from_secs(12) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if let Some(e) = audio.error() {
                    return Err(e);
                }
                if !audio.finished() {
                    return Err("Audio playback timeout".into());
                }
                println!(
                    "WASAPI preview passed; prepared in {preparation} ms; played {:.3} s",
                    audio.position()
                );
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        InitCommonControls();
        register("QuietCut.Editor", Some(proc));
        let mut app = Box::new(App::new());
        let hwnd = window(
            "QuietCut.Editor",
            "QuietCut 1.8",
            1000,
            740,
            null_mut(),
            (&mut *app) as *mut _ as *const _,
        );
        if hwnd.is_null() {
            std::process::exit(1);
        }
        let render = arguments.get(1).is_some_and(|a| a == "--render-test");
        let layout_test = arguments.get(1).is_some_and(|a| a == "--layout-test");
        let project_test = arguments.get(1).is_some_and(|a| a == "--project-test");
        let export_cancel_test = arguments
            .get(1)
            .is_some_and(|a| a == "--export-cancel-test");
        let export_test = export_cancel_test
            || arguments
                .get(1)
                .is_some_and(|a| a == "--export-window-test");
        let mut export_test_done = false;
        let mut cancel_test_sent = false;
        let mut first_progress_seen = false;
        let test_silence = arguments.get(4).is_some_and(|a| a == "silence");
        if export_test {
            let input = PathBuf::from(arguments.get(2).expect("Test video"));
            let folder = PathBuf::from(arguments.get(3).expect("Test export directory"));
            fs::create_dir_all(&folder).unwrap();
            app.timeline
                .add(probe(app.tools.as_ref().unwrap(), &input, &app.cancel).unwrap());
            app.settings.quality = 2;
            app.settings.level = true;
            app.settings.ask_save = false;
            app.settings.output_dir = Some(folder);
            app.silence = test_silence;
            app.settings.silence_preset(1);
            app.plan_ready = !test_silence;
            app.prepared = (!test_silence).then(|| PreviewPlan::uncut(&app.timeline));
            app.enabled();
            app.dialog = export_ui::ExportUi::open(
                hwnd,
                app.settings.clone(),
                app.silence,
                app.export_state(),
            );
            ShowWindow(app.dialog, SW_HIDE);
            if !export_cancel_test {
                let protected =
                    PathBuf::from(arguments.get(3).unwrap()).join("Renamed 视频 lecture.mp4");
                fs::write(&protected, b"Keep existing output").unwrap();
                export_ui::ExportUi::set_file_name_test(app.dialog, "Renamed 视频 lecture.mp4");
            }
            SendMessageW(app.dialog, WM_COMMAND, 208, 0);
        } else if project_test {
            DISPATCHING.with(|b| b.set(true));
            {
                let _guard = Guard;
                let input = PathBuf::from(arguments.get(2).expect("Project test input"));
                let folder = PathBuf::from(arguments.get(3).expect("Project test folder"));
                fs::create_dir_all(&folder).unwrap();
                let source_bytes = fs::metadata(&input).unwrap().len();
                let completed = folder.join("completed.mp4");
                fs::copy(&input, &completed).unwrap();
                for from_export in [false, true] {
                    app.timeline
                        .add(probe(app.tools.as_ref().unwrap(), &input, &app.cancel).unwrap());
                    app.history.push(app.timeline.clone());
                    app.selected = Some(0);
                    app.playhead = 1.;
                    app.scroll = 7.;
                    app.prepared = Some(PreviewPlan::uncut(&app.timeline));
                    app.plan_ready = true;
                    app.output = Some(Exported {
                        path: completed.clone(),
                        duration: 4.,
                        size: source_bytes,
                    });
                    app.export_name = Some("Custom name".into());
                    let duration = app.timeline.duration();
                    let generation = app.analysis_generation;
                    if from_export {
                        app.dialog = export_ui::ExportUi::open(
                            hwnd,
                            app.settings.clone(),
                            app.silence,
                            app.export_state(),
                        );
                        ShowWindow(app.dialog, SW_HIDE);
                    }
                    project_confirmation_test(&mut app, IDNO);
                    assert_eq!(app.timeline.duration(), duration);
                    assert!(app.output.is_some() && !app.history.is_empty() && app.plan_ready);
                    if from_export {
                        assert!(IsWindow(app.dialog) != 0);
                    }
                    project_confirmation_test(&mut app, IDYES);
                    assert!(app.timeline.clips.is_empty() && app.history.is_empty());
                    assert!(
                        app.output.is_none() && app.prepared.is_none() && app.preview.is_none()
                    );
                    assert!(app.audio.is_none() && app.audio_next.is_none());
                    assert!(app.export_name.is_none() && app.selected.is_none());
                    assert_eq!(app.playhead, 0.);
                    assert_eq!(app.scroll, 0.);
                    assert!(app.analysis_generation > generation);
                    assert!(app.dialog.is_null());
                    assert_eq!(fs::metadata(&input).unwrap().len(), source_bytes);
                    assert_eq!(fs::metadata(&completed).unwrap().len(), source_bytes);
                    println!(
                        "New project confirmed/cancelled correctly from {}",
                        if from_export { "Export" } else { "main window" }
                    );
                }
            }
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else if layout_test {
            let folder = PathBuf::from(arguments.get(2).expect("Layout output folder"));
            fs::create_dir_all(&folder).unwrap();
            verify_native_layout(&app);
            // Exercise the real Windows command and nested native message path.
            // Calling App::layout explicitly here would hide this regression.
            SendMessageW(hwnd, WM_SYSCOMMAND, SC_MAXIMIZE as usize, 0);
            ShowWindow(hwnd, SW_HIDE);
            assert!(IsZoomed(hwnd) != 0);
            verify_native_layout(&app);
            render_test(app.ui(), &folder.join("maximized-empty.bmp"), |dc, w, h| {
                app.paint(dc, w, h)
            })
            .unwrap();
            SendMessageW(hwnd, WM_SYSCOMMAND, SC_RESTORE as usize, 0);
            ShowWindow(hwnd, SW_HIDE);
            assert!(IsZoomed(hwnd) == 0);
            verify_native_layout(&app);
            for (width, height) in [(1450, 1000), (1020, 700), (1920, 1040), (1000, 660)] {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                verify_native_layout(&app);
            }
            render_test(app.ui(), &folder.join("minimum-empty.bmp"), |dc, w, h| {
                app.paint(dc, w, h)
            })
            .unwrap();
            app.ui.as_mut().unwrap().theme(false);
            render_test(app.ui(), &folder.join("light-empty.bmp"), |dc, w, h| {
                app.paint(dc, w, h)
            })
            .unwrap();
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else if render {
            let folder = PathBuf::from(arguments.get(2).expect("Render output folder"));
            let _ = fs::create_dir_all(&folder);
            DISPATCHING.with(|b| b.set(true));
            {
                let _guard = Guard;
                assert!(GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_MAXIMIZEBOX != 0);
                app.timeline.add(Media {
                    path: "Lecture.mp4".into(),
                    duration: 180.,
                    size: 120 * 1024 * 1024,
                    width: 1920,
                    height: 1080,
                    video_codec: "h264".into(),
                    audio_count: 1,
                    pixel_format: "yuv420p".into(),
                    fps: 30.,
                    start_time: 0.,
                    color: vec![],
                    audio_specs: vec![(48000, 2)],
                });
                app.timeline.split(70.);
                app.selected = Some(0);
                app.output = Some(Exported {
                    path: "Lecture-edited.mp4".into(),
                    duration: 126.4,
                    size: 48 * 1024 * 1024,
                });
                app.prepared = Some(PreviewPlan::uncut(&app.timeline));
                app.status = "Export complete".into();
                app.fit();
                app.enabled();
                render_test(app.ui(), &folder.join("editor-normal.bmp"), |dc, w, h| {
                    app.paint(dc, w, h)
                })
                .unwrap();
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    0,
                    0,
                    1450,
                    1000,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                app.layout();
                app.fit();
                render_test(app.ui(), &folder.join("editor-large.bmp"), |dc, w, h| {
                    app.paint(dc, w, h)
                })
                .unwrap();
                export_ui::ExportUi::render_test(hwnd, app.settings.clone(), &folder);
            }
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else if std::env::args_os().any(|a| a == "--smoke-test") {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else {
            ShowWindow(hwnd, SW_SHOW);
        }
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            let active = if !app.dialog.is_null() && IsWindow(app.dialog) != 0 {
                app.dialog
            } else {
                hwnd
            };
            if msg.message == WM_KEYDOWN
                && (msg.wParam as u16 == VK_SPACE
                    || msg.wParam as u16 == VK_DELETE
                    || GetKeyState(VK_CONTROL as i32) < 0)
                && active == hwnd
            {
                SendMessageW(hwnd, WM_KEYDOWN, msg.wParam, msg.lParam);
                continue;
            }
            if IsDialogMessageW(active, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if export_test && !first_progress_seen && app.progress > 0.1801 {
                first_progress_seen = true;
                let elapsed = app.export_started.unwrap().elapsed().as_secs_f64();
                println!("First GUI rendering progress after {elapsed:.3}s");
                assert!(elapsed < 15., "Render startup stalled: {elapsed:.3}s");
            }
            if export_cancel_test
                && !cancel_test_sent
                && app.busy
                && app.progress > 0.20
                && export_ui::ExportUi::verify_progress_test(
                    app.dialog,
                    &PathBuf::from(arguments.get(3).unwrap()),
                )
            {
                cancel_test_sent = true;
                println!("Cancelling an active encode at {:.1}%", app.progress * 100.);
                PostMessageW(app.dialog, WM_CLOSE, 0, 0);
            }
            if export_cancel_test
                && cancel_test_sent
                && !app.busy
                && IsWindow(app.dialog) == 0
                && !export_test_done
            {
                assert!(app.output.is_none());
                assert!(app.status.starts_with("Cancelled"));
                export_test_done = true;
                println!("Export window close cancelled the job and completed cleanup");
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            if export_test && !export_cancel_test && !export_test_done && app.output.is_some() {
                let folder = PathBuf::from(arguments.get(3).unwrap());
                if export_ui::ExportUi::verify_result_test(app.dialog, &folder) {
                    export_test_done = true;
                    PostMessageW(app.dialog, WM_CLOSE, 0, 0);
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            }
        }
        app.analysis_cancel.store(true, Ordering::Release);
        if export_test {
            assert!(export_test_done, "Export window did not receive its result");
        }
        if !render
            && !layout_test
            && !export_test
            && !project_test
            && !arguments.iter().any(|a| a == "--smoke-test")
        {
            save_settings(&app.settings);
        }
    }
}
