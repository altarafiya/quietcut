#![allow(unsafe_op_in_unsafe_fn)]
use crate::ui::*;
use quietcut::engine::*;
use quietcut::workflow::{file_stem, short_duration};
use std::{path::PathBuf, ptr::null_mut};
pub const EXPORT_SETTINGS: u32 = WM_APP + 20;
pub const PREVIEW_SETTINGS: u32 = WM_APP + 22;
pub const PREVIEW_PLAY: u32 = WM_APP + 23;
pub const EXPORT_UPDATE: u32 = WM_APP + 24;
pub const EXPORT_CANCEL: u32 = WM_APP + 25;
const OPEN_RESULT: usize = 214;
const FILE_NAME: usize = 215;
const NEW_PROJECT: usize = 216;
const SILENCE: usize = 201;
const THRESHOLD: usize = 203;
const MINIMUM: usize = 204;
const PADDING: usize = 205;
const AUDIO: usize = 206;
const SAVE: usize = 208;
const BACK: usize = 209;
const PREVIEW: usize = 210;
const SOURCE_FOLDER: usize = 211;
const PICK_FOLDER: usize = 212;
const ASK_FOLDER: usize = 213;
const PRESETS: [(&str, &str); 4] = [
    ("\u{e995}", "Gentle"),
    ("\u{e9d9}", "Balanced"),
    ("\u{e945}", "Tight"),
    ("\u{e713}", "Custom"),
];
const QUALITIES: [(&str, &str); 4] = [
    ("\u{e8c8}", "Fast copy"),
    ("\u{e73e}", "Lossless"),
    ("\u{e768}", "High quality"),
    ("\u{e74e}", "Compact"),
];
#[derive(Clone)]
pub struct ExportRequest {
    pub settings: Settings,
    pub silence: bool,
    pub file_name: Option<String>,
}
#[derive(Clone, Default)]
pub struct ExportState {
    pub file_name: String,
    pub elapsed: f64,
    pub remaining: Option<f64>,
    pub before_duration: f64,
    pub before_size: u64,
    pub after_duration: Option<f64>,
    pub output: Option<Exported>,
    pub busy: bool,
    pub progress: f64,
    pub playing: bool,
    pub status: String,
}
pub struct ExportUi {
    ui: Option<Ui>,
    owner: HWND,
    settings: Settings,
    silence: bool,
    preset: usize,
    state: ExportState,
    close_pending: bool,
}
impl ExportUi {
    pub unsafe fn set_file_name_test(hwnd: HWND, value: &str) {
        let state = &*(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Self);
        state.ui().set_text(FILE_NAME, value);
    }
    pub unsafe fn verify_progress_test(hwnd: HWND, folder: &std::path::Path) -> bool {
        let state = &*(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Self);
        if !state.state.busy || state.state.progress <= 0.20 || state.state.remaining.is_none() {
            return false;
        }
        assert!(state.state.elapsed > 0.);
        assert!(state.state.remaining.unwrap() >= 0.);
        assert!(IsWindowEnabled(state.ui().get(SAVE)) == 0);
        assert!(IsWindowEnabled(state.ui().get(FILE_NAME)) == 0);
        assert!(GetWindowLongW(state.ui().get(NEW_PROJECT), GWL_STYLE) as u32 & WS_VISIBLE == 0);
        crate::ui::render_test(state.ui(), &folder.join("export-active.bmp"), |dc, w, h| {
            state.paint(dc, w, h)
        })
        .unwrap();
        println!(
            "Popup progress verified: {:.1}%, elapsed {:.2}s, estimated {:.1}s remaining",
            state.state.progress * 100.,
            state.state.elapsed,
            state.state.remaining.unwrap()
        );
        true
    }
    pub unsafe fn verify_result_test(hwnd: HWND, folder: &std::path::Path) -> bool {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Self;
        if ptr.is_null() {
            return false;
        }
        let state = &*ptr;
        let Some(output) = &state.state.output else {
            return false;
        };
        assert!(!state.state.busy);
        assert_eq!(std::fs::metadata(&output.path).unwrap().len(), output.size);
        assert!(output.duration > 0.);
        assert!(IsWindowEnabled(state.ui().get(SAVE)) != 0);
        assert!(
            output
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Renamed 视频 lecture-")
        );
        assert_eq!(
            std::fs::read(folder.join("Renamed 视频 lecture.mp4")).unwrap(),
            b"Keep existing output"
        );
        for id in [OPEN_RESULT, NEW_PROJECT] {
            assert!(GetWindowLongW(state.ui().get(id), GWL_STYLE) as u32 & WS_VISIBLE != 0);
            assert!(IsWindowEnabled(state.ui().get(id)) != 0);
        }
        crate::ui::render_test(
            state.ui(),
            &folder.join("export-actual-result.bmp"),
            |dc, w, h| state.paint(dc, w, h),
        )
        .unwrap();
        println!(
            "Export window verified actual result: {} bytes, {:.3} seconds",
            output.size, output.duration
        );
        true
    }
    pub unsafe fn render_test(owner: HWND, mut settings: Settings, folder: &std::path::Path) {
        settings.exact_silence(true);
        register("QuietCut.Export", Some(proc));
        let ptr = Box::into_raw(Box::new(Self {
            ui: None,
            owner,
            settings,
            silence: true,
            preset: 1,
            state: ExportState {
                before_duration: 180.,
                file_name: "Lecture-edited".into(),
                before_size: 120 * 1024 * 1024,
                after_duration: Some(126.4),
                ..ExportState::default()
            },
            close_pending: false,
        }));
        let hwnd = window(
            "QuietCut.Export",
            "Export video",
            940,
            584,
            owner,
            ptr as *const _,
        );
        if hwnd.is_null() {
            drop(Box::from_raw(ptr));
            return;
        }
        {
            BUSY.with(|b| b.set(true));
            let _guard = Guard;
            let state = &mut *ptr;
            assert_ne!(state.settings.quality, 0);
            assert!(IsWindowEnabled(state.ui().get(230)) == 0);
            let _ =
                crate::ui::render_test(state.ui(), &folder.join("export-icons.bmp"), |dc, w, h| {
                    state.paint(dc, w, h)
                });
            state.state.busy = true;
            state.state.progress = 0.6;
            state.state.elapsed = 38.;
            state.state.remaining = Some(27.);
            state.state.status = "NVIDIA GPU · clip 3 / 5".into();
            state.sync();
            assert!(IsWindowEnabled(state.ui().get(SAVE)) == 0);
            crate::ui::render_test(
                state.ui(),
                &folder.join("export-progress.bmp"),
                |dc, w, h| state.paint(dc, w, h),
            )
            .unwrap();
            state.state.busy = false;
            state.state.output = Some(Exported {
                path: "Lecture-edited.mp4".into(),
                duration: 126.4,
                size: 48 * 1024 * 1024,
            });
            state.state.status = "Export complete".into();
            state.sync();
            assert!(IsWindowEnabled(state.ui().get(SAVE)) != 0);
            assert!(
                GetWindowLongW(state.ui().get(OPEN_RESULT), GWL_STYLE) as u32 & WS_VISIBLE != 0
            );
            crate::ui::render_test(state.ui(), &folder.join("export-result.bmp"), |dc, w, h| {
                state.paint(dc, w, h)
            })
            .unwrap();
            state.preset = 3;
            state.settings.adaptive = false;
            state.sync();
            state.layout();
            let _ = crate::ui::render_test(
                state.ui(),
                &folder.join("export-custom.bmp"),
                |dc, w, h| state.paint(dc, w, h),
            );
        }
        DestroyWindow(hwnd);
    }
    pub unsafe fn open(
        owner: HWND,
        mut settings: Settings,
        silence: bool,
        state: ExportState,
    ) -> HWND {
        settings.exact_silence(silence);
        register("QuietCut.Export", Some(proc));
        let preset = if !settings.adaptive {
            3
        } else if settings.min_silence >= 0.75 {
            0
        } else if settings.min_silence >= 0.45 {
            1
        } else {
            2
        };
        let ptr = Box::into_raw(Box::new(Self {
            ui: None,
            owner,
            settings,
            silence,
            preset,
            state,
            close_pending: false,
        }));
        let h = window(
            "QuietCut.Export",
            "Export video",
            940,
            584,
            owner,
            ptr as *const _,
        );
        if h.is_null() {
            drop(Box::from_raw(ptr));
            return h;
        }
        EnableWindow(owner, 0);
        ShowWindow(h, SW_SHOW);
        h
    }
    fn ui(&self) -> &Ui {
        self.ui.as_ref().unwrap()
    }
    unsafe fn init(&mut self, hwnd: HWND) {
        let mut u = Ui::new(hwnd, self.settings.dark);
        for id in [SILENCE, AUDIO, SOURCE_FOLDER, PICK_FOLDER, ASK_FOLDER] {
            u.button(id, "");
        }
        u.button(PREVIEW, "\u{e768}|Preview");
        for i in 0..4 {
            u.button(220 + i, "");
            u.button(230 + i, "");
        }
        u.slider(THRESHOLD, 5, 80, (-self.settings.threshold) as i32);
        u.slider(MINIMUM, 5, 300, (self.settings.min_silence * 100.) as i32);
        u.slider(PADDING, 0, 100, (self.settings.padding * 100.) as i32);
        u.button(BACK, "Back");
        u.button(SAVE, "!\u{e898}|Export");
        u.button(OPEN_RESULT, "\u{e8b7}|Open folder");
        u.button(NEW_PROJECT, "\u{e710}|New project");
        u.edit(FILE_NAME, &self.state.file_name);
        self.ui = Some(u);
        self.sync();
        self.layout();
    }
    unsafe fn emit(&self) {
        PostMessageW(
            self.owner,
            PREVIEW_SETTINGS,
            Box::into_raw(Box::new(ExportRequest {
                settings: self.settings.clone(),
                silence: self.silence,
                file_name: None,
            })) as usize,
            0,
        );
    }
    unsafe fn sync(&self) {
        let u = self.ui();
        for (id, _) in &u.controls {
            u.enable(*id, !self.state.busy || *id == BACK);
        }
        u.enable(230, !self.state.busy && !self.silence);
        u.set_text(BACK, if self.state.busy { "Cancel" } else { "Back" });
        u.set_text(
            PREVIEW,
            if self.state.playing {
                "\u{e769}|Pause"
            } else {
                "\u{e768}|Preview"
            },
        );
        u.show(OPEN_RESULT, self.state.output.is_some() && !self.state.busy);
        u.show(NEW_PROJECT, self.state.output.is_some() && !self.state.busy);
        u.set_text(
            SILENCE,
            if self.silence {
                "~\u{e8c6}|Remove silence:on"
            } else {
                "~\u{e8c6}|Remove silence:off"
            },
        );
        u.set_text(
            AUDIO,
            if self.settings.level {
                "~\u{e767}|Even audio:on"
            } else {
                "~\u{e767}|Even audio:off"
            },
        );
        for (i, (icon, label)) in PRESETS.iter().enumerate() {
            u.set_text(
                220 + i,
                &format!("^{}{icon}|{label}", if self.preset == i { "!" } else { "" }),
            );
        }
        for (i, (icon, label)) in QUALITIES.iter().enumerate() {
            u.set_text(
                230 + i,
                &format!(
                    "^{}{icon}|{label}",
                    if self.settings.quality == i { "!" } else { "" }
                ),
            );
        }
        u.set_text(
            SOURCE_FOLDER,
            &format!(
                "{}\u{e8b7}|Video folder",
                if !self.settings.ask_save && self.settings.output_dir.is_none() {
                    "="
                } else {
                    ""
                }
            ),
        );
        u.set_text(
            PICK_FOLDER,
            &format!(
                "{}\u{e838}|Choose folder",
                if !self.settings.ask_save && self.settings.output_dir.is_some() {
                    "="
                } else {
                    ""
                }
            ),
        );
        u.set_text(
            ASK_FOLDER,
            &format!(
                "{}\u{e8a7}|Ask each time",
                if self.settings.ask_save { "=" } else { "" }
            ),
        );
        u.refresh();
    }
    fn custom(&self) -> bool {
        self.silence && self.preset == 3
    }
    fn audio_y(&self) -> i32 {
        if !self.silence {
            112
        } else if self.custom() {
            300
        } else {
            200
        }
    }
    unsafe fn layout(&self) {
        let u = self.ui();
        let y = self.audio_y();
        u.place(SILENCE, 24, 66, 592, 34);
        for i in 0..4 {
            u.show(220 + i, self.silence);
            u.place(220 + i, 24 + i as i32 * 151, 108, 139, 70);
            u.place(230 + i, 24 + i as i32 * 151, y + 78, 139, 70);
        }
        for (i, id) in [THRESHOLD, MINIMUM, PADDING].into_iter().enumerate() {
            u.show(id, self.custom());
            u.place(id, 24 + i as i32 * 203, 244, 186, 30);
        }
        u.place(AUDIO, 24, y, 404, 34);
        u.place(PREVIEW, 456, y, 160, 36);
        u.place(SOURCE_FOLDER, 24, y + 223, 166, 36);
        u.place(PICK_FOLDER, 202, y + 223, 226, 36);
        u.place(ASK_FOLDER, 440, y + 223, 176, 36);
        u.place(FILE_NAME, 24, y + 333, 500, 36);
        let height = y + 448;
        u.place(BACK, 24, height - 62, 92, 40);
        u.place(SAVE, 426, height - 62, 190, 40);
        u.place(OPEN_RESULT, 666, height - 112, 250, 40);
        u.place(NEW_PROJECT, 666, height - 62, 250, 40);
        let mut r = RECT {
            left: 0,
            top: 0,
            right: u.s(940),
            bottom: u.s(height),
        };
        AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            0,
            0,
            (u.dpi * 96.) as u32,
        );
        let mut current: RECT = std::mem::zeroed();
        GetWindowRect(u.hwnd, &mut current);
        let mut monitor: MONITORINFO = std::mem::zeroed();
        monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(
            MonitorFromWindow(u.hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        let x = current.left.clamp(
            monitor.rcWork.left,
            (monitor.rcWork.right - (r.right - r.left)).max(monitor.rcWork.left),
        );
        let y = current.top.clamp(
            monitor.rcWork.top,
            (monitor.rcWork.bottom - (r.bottom - r.top)).max(monitor.rcWork.top),
        );
        SetWindowPos(
            u.hwnd,
            null_mut(),
            x,
            y,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        u.refresh();
    }
    unsafe fn paint(&self, dc: HDC, width: i32, height: i32) {
        let u = self.ui();
        let p = u.colors;
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
        u.label(dc, "Export", 24, 18, 450, 36, p.fg, 1);
        let y = self.audio_y();
        if self.custom() {
            for (i, (label, value)) in [
                ("Threshold", format!("{} dB", self.settings.threshold)),
                (
                    "Min. silence",
                    format!("{:.2} s", self.settings.min_silence),
                ),
                ("Keep edges", format!("{:.2} s", self.settings.padding)),
            ]
            .into_iter()
            .enumerate()
            {
                let x = 24 + i as i32 * 203;
                u.label(dc, label, x, 194, 186, 22, p.muted, 0);
                u.label(dc, &value, x, 218, 186, 22, p.fg, 0);
            }
        }
        u.label(dc, "QUALITY", 24, y + 50, 592, 22, p.muted, 0);
        u.label(
            dc,
            match self.settings.quality {
                0 => "Original streams · cuts snap to keyframes",
                1 => "Exact cuts · no pixel loss · GPU auto",
                2 => "Exact cuts · high quality · GPU auto",
                _ => "Exact cuts · compact · GPU auto",
            },
            24,
            y + 156,
            592,
            24,
            p.muted,
            0,
        );
        u.label(dc, "SAVE TO", 24, y + 195, 592, 22, p.muted, 0);
        if !self.settings.ask_save {
            u.label(
                dc,
                &self
                    .settings
                    .output_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Beside the first video".into()),
                24,
                y + 269,
                592,
                24,
                p.fg,
                0,
            );
        }
        u.label(dc, "FILE NAME", 24, y + 305, 592, 22, p.muted, 0);
        u.rect(dc, 536, y + 333, 80, 36, 7, p.field, p.line);
        u.label(
            dc,
            &format!(".{}", self.settings.extension()),
            548,
            y + 333,
            60,
            36,
            p.muted,
            0,
        );
        u.rect(
            dc,
            640,
            66,
            1,
            (height as f64 / u.dpi) as i32 - 90,
            0,
            p.line,
            p.line,
        );
        for y in [66, 226] {
            u.rect(dc, 666, y, 250, 112, 9, p.panel, p.line);
        }
        u.label(dc, "BEFORE", 682, 77, 218, 20, p.muted, 0);
        u.label(
            dc,
            &human_time(self.state.before_duration),
            682,
            101,
            218,
            34,
            p.fg,
            1,
        );
        u.label(
            dc,
            &human_size(self.state.before_size),
            682,
            143,
            218,
            22,
            p.muted,
            0,
        );
        u.label(dc, "↓", 772, 183, 42, 36, p.accent, 1);
        u.label(dc, "AFTER", 682, 237, 218, 20, p.muted, 0);
        let duration = self
            .state
            .output
            .as_ref()
            .map(|o| human_time(o.duration))
            .or_else(|| {
                self.state
                    .after_duration
                    .map(|d| format!("~{}", human_time(d)))
            })
            .unwrap_or_else(|| "Analyzing…".into());
        u.label(
            dc,
            &duration,
            682,
            261,
            218,
            34,
            p.fg,
            if duration == "Analyzing…" { 2 } else { 1 },
        );
        let size = self
            .state
            .output
            .as_ref()
            .map(|o| human_size(o.size))
            .unwrap_or_else(|| "Size after export".into());
        u.label(dc, &size, 682, 303, 218, 22, p.muted, 0);
        if self.state.busy || self.state.output.is_some() {
            u.rect(dc, 666, 358, 250, 6, 3, p.line, p.line);
            if self.state.busy && self.state.progress <= 0.18 && self.state.remaining.is_none() {
                let x = 666 + ((self.state.elapsed * 70.) as i32 % 190);
                u.rect(dc, x, 358, 60, 6, 3, p.accent, p.accent);
            } else {
                u.rect(
                    dc,
                    666,
                    358,
                    (250. * self.state.progress.clamp(0., 1.)) as i32,
                    6,
                    3,
                    p.accent,
                    p.accent,
                );
            }
            let elapsed = short_duration(self.state.elapsed.floor());
            let label = if self.state.busy {
                format!(
                    "{:.0}%  ·  Elapsed {elapsed}",
                    self.state.progress.clamp(0., 1.) * 100.
                )
            } else {
                format!("Done  ·  {elapsed}")
            };
            u.label(dc, &label, 666, 370, 250, 24, p.fg, 0);
            if self.state.busy {
                let eta = self
                    .state
                    .remaining
                    .map(|s| format!("About {} remaining", short_duration(s)))
                    .unwrap_or_else(|| "Estimating time…".into());
                u.label(dc, &eta, 666, 396, 250, 24, p.muted, 0);
            }
        }
        u.label(dc, &self.state.status, 666, 422, 250, 22, p.muted, 0);
    }
    unsafe fn choose_folder(&self) -> Option<PathBuf> {
        use windows::Win32::{System::Com::*, UI::Shell::*};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let d: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        d.SetOptions(d.GetOptions().ok()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        d.Show(Some(windows::Win32::Foundation::HWND(self.ui().hwnd)))
            .ok()?;
        let item = d.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let result = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const _));
        result
    }
    unsafe fn command(&mut self, id: usize) {
        if self.state.busy {
            if id == BACK {
                PostMessageW(self.owner, EXPORT_CANCEL, 0, 0);
            }
            return;
        }
        match id {
            SILENCE => {
                self.silence = !self.silence;
                self.layout();
            }
            AUDIO => self.settings.level = !self.settings.level,
            220..=223 => {
                self.preset = id - 220;
                self.settings.silence_preset(self.preset);
                for (id, value) in [
                    (THRESHOLD, -self.settings.threshold),
                    (MINIMUM, self.settings.min_silence * 100.),
                    (PADDING, self.settings.padding * 100.),
                ] {
                    SendMessageW(self.ui().get(id), TBM_SETPOS, 1, value as isize);
                }
                self.layout();
            }
            230..=233 => {
                if id == 230 && self.silence {
                    return;
                }
                self.settings.quality = id - 230;
            }
            SOURCE_FOLDER => {
                self.settings.output_dir = None;
                self.settings.ask_save = false;
            }
            PICK_FOLDER => {
                if let Some(path) = self.choose_folder() {
                    self.settings.output_dir = Some(path);
                    self.settings.ask_save = false;
                }
            }
            ASK_FOLDER => self.settings.ask_save = true,
            PREVIEW => {
                PostMessageW(self.owner, PREVIEW_PLAY, 0, 0);
                return;
            }
            BACK => {
                DestroyWindow(self.ui().hwnd);
                return;
            }
            SAVE => {
                let name = match file_stem(&self.ui().value(FILE_NAME)) {
                    Ok(name) => name,
                    Err(error) => {
                        MessageBoxW(
                            self.ui().hwnd,
                            w(&error).as_ptr(),
                            w("File name").as_ptr(),
                            MB_OK | MB_ICONWARNING,
                        );
                        SetFocus(self.ui().get(FILE_NAME));
                        return;
                    }
                };
                self.ui().set_text(FILE_NAME, &name);
                PostMessageW(
                    self.owner,
                    EXPORT_SETTINGS,
                    Box::into_raw(Box::new(ExportRequest {
                        settings: self.settings.clone(),
                        silence: self.silence,
                        file_name: Some(name),
                    })) as usize,
                    0,
                );
                return;
            }
            OPEN_RESULT => {
                if let Some(output) = &self.state.output {
                    ShellExecuteW(
                        self.ui().hwnd,
                        w("open").as_ptr(),
                        w(&output
                            .path
                            .parent()
                            .unwrap_or(&output.path)
                            .to_string_lossy())
                        .as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOWNORMAL,
                    );
                }
                return;
            }
            NEW_PROJECT => {
                PostMessageW(self.owner, WM_COMMAND, crate::NEW_PROJECT, 0);
                return;
            }
            _ => return,
        }
        self.settings.exact_silence(self.silence);
        self.sync();
        self.emit();
    }
}
thread_local! {static BUSY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        BUSY.with(|b| b.set(false));
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(v) = early(hwnd, msg, wp, lp) {
        return v;
    }
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ExportUi;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_NCDESTROY {
        let owner = GetWindow(hwnd, GW_OWNER);
        EnableWindow(owner, 1);
        PostMessageW(owner, WM_APP + 21, ptr as usize, 0);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if BUSY.with(|b| b.replace(true)) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let _guard = Guard;
    let a = &mut *ptr;
    match msg {
        WM_CREATE => {
            a.init(hwnd);
            0
        }
        WM_COMMAND => {
            a.command(wp & 0xffff);
            0
        }
        WM_HSCROLL => {
            a.settings.threshold = -(a.ui().pos(THRESHOLD) as f64);
            a.settings.min_silence = a.ui().pos(MINIMUM) as f64 / 100.;
            a.settings.padding = a.ui().pos(PADDING) as f64 / 100.;
            a.ui().refresh();
            a.emit();
            0
        }
        WM_PAINT => {
            paint(hwnd, |dc, w, h| a.paint(dc, w, h));
            0
        }
        EXPORT_UPDATE => {
            a.state = *Box::from_raw(wp as *mut ExportState);
            a.sync();
            if a.close_pending && !a.state.busy {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_CLOSE => {
            if a.state.busy {
                a.close_pending = true;
                PostMessageW(a.owner, EXPORT_CANCEL, 0, 0);
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
