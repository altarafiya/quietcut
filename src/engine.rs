//! Native FFmpeg pipeline. No shell interpolation; all arguments are passed separately.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "fast_export.rs"]
mod fast_export;

pub type Result<T> = std::result::Result<T, String>;
pub type Cancel = Arc<AtomicBool>;
// Causal leveling: no future-window gain smoothing or pre-cut attenuation.
// Segmented edits reset per span; the one-pass silence path processes original
// pauses first, allowing recovery before discarding them. No future-window gain smoothing.
pub const LEVEL_FILTER: &str = "acompressor=threshold=0.02:ratio=20:attack=2:release=30:makeup=8:knee=2:link=maximum:detection=rms,alimiter=limit=0.95:attack=1:release=10:level=0:latency=1";

#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl Tools {
    pub fn find() -> Result<Self> {
        let mut dirs = Vec::new();
        if let Some(p) = std::env::var_os("QUIETCUT_FFMPEG") {
            dirs.push(PathBuf::from(p));
        }
        if let Ok(p) = std::env::current_exe() {
            if let Some(p) = p.parent() {
                dirs.push(p.join("tools"));
                dirs.push(p.to_owned());
            }
        }
        if let Some(p) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&p));
        }
        #[cfg(windows)]
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            let base = PathBuf::from(p).join("Microsoft/WinGet/Packages");
            if let Ok(entries) = fs::read_dir(base) {
                for e in entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().contains("FFmpeg"))
                {
                    dirs.push(e.path().join("bin"));
                    if let Ok(children) = fs::read_dir(e.path()) {
                        dirs.extend(children.flatten().map(|e| e.path().join("bin")));
                    }
                }
            }
        }
        for dir in dirs {
            let ffmpeg = dir.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
            let ffprobe = dir.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
            if ffmpeg.is_file() && ffprobe.is_file() {
                return Ok(Self { ffmpeg, ffprobe });
            }
        }
        Err("FFmpeg was not found. Place ffmpeg and ffprobe in a tools folder beside QuietCut, or add them to PATH.".into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub threshold: f64,
    pub min_silence: f64,
    pub padding: f64,
    /// 0 = copy; 1 = FFV1 lossless; 2 = H.264 CRF18; 3 = H.264 CRF23
    pub quality: usize,
    pub level: bool,
    pub dark: bool,
    #[serde(default = "default_gpu")]
    pub gpu: bool,
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
    #[serde(default = "default_gpu")]
    pub ask_save: bool,
    #[serde(default = "default_gpu")]
    pub adaptive: bool,
}
fn default_gpu() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            threshold: -50.0,
            min_silence: 0.5,
            padding: 0.14,
            quality: 0,
            level: false,
            dark: true,
            gpu: true,
            output_dir: None,
            ask_save: true,
            adaptive: true,
        }
    }
}
impl Settings {
    /// Stream copying cannot remove pauses shorter than a source GOP.
    /// The GUI selects GPU-backed exact cuts whenever silence removal is on.
    pub fn exact_silence(&mut self, silence: bool) {
        if silence && self.quality == 0 {
            self.quality = 2;
        }
    }
    pub fn silence_preset(&mut self, preset: usize) {
        self.adaptive = preset != 3;
        match preset {
            0 => {
                self.threshold = -55.;
                self.min_silence = 0.8;
                self.padding = 0.18;
            }
            1 => {
                self.threshold = -50.;
                self.min_silence = 0.5;
                self.padding = 0.14;
            }
            2 => {
                self.threshold = -50.;
                self.min_silence = 0.3;
                self.padding = 0.12;
            }
            _ => {}
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !self.threshold.is_finite() || !(-80.0..=-5.0).contains(&self.threshold) {
            return Err("Threshold must be between -80 and -5 dB.".into());
        }
        if !self.min_silence.is_finite() || !(0.05..=30.0).contains(&self.min_silence) {
            return Err("Minimum silence must be between 0.05 and 30 seconds.".into());
        }
        if !self.padding.is_finite() || !(0.0..=2.0).contains(&self.padding) {
            return Err("Keep edges must be between 0 and 2 seconds.".into());
        }
        if self.quality > 3 {
            return Err("Unknown quality setting.".into());
        }
        Ok(())
    }
    pub fn detection_eq(&self, other: &Self) -> bool {
        self.threshold == other.threshold
            && self.min_silence == other.min_silence
            && self.padding == other.padding
            && self.adaptive == other.adaptive
            && (self.quality == 0) == (other.quality == 0)
    }
    pub fn extension(&self) -> &'static str {
        if self.quality >= 2 { "mp4" } else { "mkv" }
    }
}

#[derive(Clone, Debug)]
pub struct Media {
    pub path: PathBuf,
    pub duration: f64,
    pub size: u64,
    pub width: u64,
    pub height: u64,
    pub video_codec: String,
    pub audio_count: usize,
    pub pixel_format: String,
    pub fps: f64,
    pub start_time: f64,
    pub color: Vec<(String, String)>,
    pub audio_specs: Vec<(u32, u32)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}
impl Span {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }
}
#[derive(Clone, Debug)]
pub struct Analysis {
    pub keep: Vec<Span>,
    pub removed: Vec<Span>,
    pub duration: f64,
    pub settings: Settings,
    pub detected: usize,
}
#[derive(Clone, Debug)]
pub struct Exported {
    pub path: PathBuf,
    pub duration: f64,
    pub size: u64,
}

fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}
fn arg(v: &mut Vec<OsString>, key: &str, value: impl Into<OsString>) {
    v.push(key.into());
    v.push(value.into());
}
fn number(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}
fn cancelled(cancel: &Cancel) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}

/// Drain both pipes concurrently, keeping memory bounded. The callback receives complete lines.
fn run(
    exe: &Path,
    argv: &[OsString],
    cancel: &Cancel,
    mut line: impl FnMut(bool, &str),
) -> Result<()> {
    run_guarded(exe, argv, cancel, None, &mut line)
}

#[derive(Default)]
struct EncoderProgress {
    frames: u64,
    micros: u64,
}
impl EncoderProgress {
    fn advance(&mut self, line: &str) -> bool {
        let Some((key, value)) = line.trim().split_once('=') else {
            return false;
        };
        let target = match key {
            "frame" => &mut self.frames,
            "out_time_us" => &mut self.micros,
            _ => return false,
        };
        if let Ok(n) = value.trim().parse::<u64>() {
            if n > *target {
                *target = n;
                return true;
            }
        }
        false
    }
}

pub(crate) fn media_command(path: &Path) -> Command {
    let command = Command::new(path);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(0x08000000);
        command
    };
    command
}

fn run_guarded(
    exe: &Path,
    argv: &[OsString],
    cancel: &Cancel,
    idle_limit: Option<Duration>,
    mut line: impl FnMut(bool, &str),
) -> Result<()> {
    cancelled(cancel)?;
    let mut child = media_command(exe)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Cannot start {}: {e}",
                exe.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;
    let (tx, rx) = mpsc::sync_channel::<(bool, String)>(128);
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let tx2 = tx.clone();
    let a = thread::spawn(move || {
        for l in BufReader::new(out)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if tx.send((true, l)).is_err() {
                break;
            }
        }
    });
    let b = thread::spawn(move || {
        for l in BufReader::new(err)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if tx2.send((false, l)).is_err() {
                break;
            }
        }
    });
    let mut tail = String::new();
    let mut stopped = false;
    let mut stalled = false;
    let mut last_advance = std::time::Instant::now();
    let mut encoder_progress = EncoderProgress::default();
    loop {
        if cancel.load(Ordering::Relaxed) && !stopped {
            let _ = child.kill();
            stopped = true;
        }
        if !stopped && !stalled && idle_limit.is_some_and(|limit| last_advance.elapsed() > limit) {
            let _ = child.kill();
            stalled = true;
        }
        match rx.recv_timeout(Duration::from_millis(30)) {
            Ok((stdout, l)) => {
                if stdout && idle_limit.is_some() && encoder_progress.advance(&l) {
                    last_advance = std::time::Instant::now();
                }
                if !stdout {
                    tail.push_str(&l);
                    tail.push('\n');
                    if tail.len() > 16000 {
                        let cut = tail
                            .char_indices()
                            .map(|(i, _)| i)
                            .find(|i| *i >= 8000)
                            .unwrap_or(tail.len());
                        tail.drain(..cut);
                    }
                }
                line(stdout, &l);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    break;
                }
                // Continue checking cancellation/deadlines if a child closes its
                // pipes but waits in driver shutdown or file finalization.
                thread::sleep(Duration::from_millis(30));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = a.join();
    let _ = b.join();
    if stopped {
        return Err("Cancelled".into());
    }
    if stalled {
        return Err("Fast export stopped making progress; retrying compatible export.".into());
    }
    if !status.success() {
        return Err(format!("Media processing failed.\n{}", tail.trim()));
    }
    Ok(())
}

pub fn probe(tools: &Tools, path: &Path, cancel: &Cancel) -> Result<Media> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let path = absolute.as_path();
    let mut argv = args(&[
        "-v",
        "error",
        "-show_format",
        "-show_streams",
        "-of",
        "json",
    ]);
    argv.push(path.as_os_str().into());
    let mut json = String::new();
    run(&tools.ffprobe, &argv, cancel, |stdout, l| {
        if stdout && json.len() < 8_000_000 {
            json.push_str(l);
            json.push('\n');
        }
    })?;
    let v: Value =
        serde_json::from_str(&json).map_err(|e| format!("Cannot read video information: {e}"))?;
    let streams = v["streams"]
        .as_array()
        .ok_or("This file contains no media streams.")?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .ok_or("This file has no video track.")?;
    let duration = number(v["format"].get("duration"))
        .or_else(|| number(video.get("duration")))
        .ok_or("This video does not have a known duration.")?;
    if duration <= 0.0 || !duration.is_finite() {
        return Err("This video has an invalid duration.".into());
    }
    let fps_text = video["avg_frame_rate"].as_str().unwrap_or("30/1");
    let fps = fps_text
        .split_once('/')
        .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(30.0);
    let color = [
        ("color_primaries", "-color_primaries"),
        ("color_transfer", "-color_trc"),
        ("color_space", "-colorspace"),
        ("color_range", "-color_range"),
    ]
    .iter()
    .filter_map(|(k, flag)| {
        video[*k]
            .as_str()
            .filter(|s| !matches!(*s, "unknown" | "unspecified" | "reserved"))
            .map(|s| (flag.to_string(), s.to_string()))
    })
    .collect();
    Ok(Media {
        path: path.to_owned(),
        duration,
        size: fs::metadata(path).map_err(|e| e.to_string())?.len(),
        width: video["width"].as_u64().unwrap_or(0),
        height: video["height"].as_u64().unwrap_or(0),
        video_codec: video["codec_name"].as_str().unwrap_or("video").to_owned(),
        audio_count: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .count(),
        pixel_format: video["pix_fmt"].as_str().unwrap_or("yuv420p").to_owned(),
        fps,
        start_time: number(v["format"].get("start_time")).unwrap_or(0.0),
        color,
        audio_specs: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .map(|s| {
                (
                    s["sample_rate"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(48000),
                    s["channels"].as_u64().unwrap_or(2) as u32,
                )
            })
            .collect(),
    })
}

fn parse_time(line: &str, key: &str) -> Option<f64> {
    line.split_once(key)?
        .1
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

pub fn cuts_from_silence(silences: &[Span], duration: f64, padding: f64) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for s in silences {
        let start = if s.start <= 0.001 {
            0.0
        } else {
            (s.start + padding).min(duration)
        };
        let end = if s.end >= duration - 0.001 {
            duration
        } else {
            (s.end - padding).max(0.0)
        };
        if end - start < 0.001 {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if start <= last.end {
                last.end = end.max(last.end);
                continue;
            }
        }
        out.push(Span { start, end });
    }
    out
}
pub fn invert(cuts: &[Span], duration: f64) -> Vec<Span> {
    let mut keep = Vec::new();
    let mut p = 0.0;
    for s in cuts {
        if s.start - p > 0.001 {
            keep.push(Span {
                start: p,
                end: s.start,
            });
        }
        p = s.end;
    }
    if duration - p > 0.001 {
        keep.push(Span {
            start: p,
            end: duration,
        });
    }
    keep
}
pub fn align_to_keys(cuts: &[Span], keys: &[f64], duration: f64) -> Vec<Span> {
    cuts.iter()
        .filter_map(|s| {
            let start = if s.start <= 0.001 {
                0.0
            } else {
                *keys.get(keys.partition_point(|k| *k < s.start))?
            };
            let end = if s.end >= duration - 0.001 {
                duration
            } else {
                let i = keys.partition_point(|k| *k <= s.end);
                if i == 0 {
                    return None;
                }
                keys[i - 1]
            };
            if end - start > 0.002 {
                Some(Span { start, end })
            } else {
                None
            }
        })
        .collect()
}

pub fn analyze(
    tools: &Tools,
    media: &Media,
    settings: &Settings,
    cancel: &Cancel,
    progress: impl FnMut(f64, &str),
) -> Result<Analysis> {
    use std::sync::{Mutex, OnceLock};
    type Cache = Vec<(String, Analysis)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    cancelled(cancel)?;
    settings.validate()?;
    let meta = fs::metadata(&media.path).map_err(|e| e.to_string())?;
    let key = format!(
        "{}|{}|{:?}|{:?}|{:?}|{:?}|{}|{}",
        media.path.display(),
        meta.len(),
        meta.modified().ok(),
        settings.threshold.to_bits(),
        settings.min_silence.to_bits(),
        settings.padding.to_bits(),
        settings.quality == 0,
        settings.adaptive
    );
    let cache = CACHE.get_or_init(|| Mutex::new(vec![]));
    if let Some((_, a)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Ok(a.clone());
    }
    let result = analyze_uncached(tools, media, settings, cancel, progress)?;
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 16 {
        cache.remove(0);
    }
    cache.push((key, result.clone()));
    Ok(result)
}
fn analyze_uncached(
    tools: &Tools,
    media: &Media,
    settings: &Settings,
    cancel: &Cancel,
    mut progress: impl FnMut(f64, &str),
) -> Result<Analysis> {
    settings.validate()?;
    if media.audio_count == 0 {
        return Err("This video has no audio track to detect silence.".into());
    }
    let mut silences = if settings.adaptive {
        crate::silence::detect(tools, media, settings, cancel, |p, s| progress(p * 0.8, s))?
    } else {
        let mut argv = args(&["-hide_banner", "-nostdin", "-nostats", "-vn", "-i"]);
        argv.push(media.path.as_os_str().into());
        // All audio tracks and channels must be quiet before any picture is removed.
        let inputs = (0..media.audio_count)
            .map(|i| format!("[0:a:{i}]"))
            .collect::<String>();
        let filter = if media.audio_count > 1 {
            format!(
                "{inputs}amerge=inputs={},silencedetect=n={}dB:d={}[detect]",
                media.audio_count, settings.threshold, settings.min_silence
            )
        } else {
            format!(
                "[0:a:0]silencedetect=n={}dB:d={}[detect]",
                settings.threshold, settings.min_silence
            )
        };
        arg(&mut argv, "-filter_complex", filter);
        argv.extend(args(&[
            "-map",
            "[detect]",
            "-progress",
            "pipe:1",
            "-f",
            "null",
            "-",
        ]));
        let mut silences = Vec::new();
        let mut start = None;
        progress(0.0, "Finding silence…");
        run(&tools.ffmpeg, &argv, cancel, |stdout, line| {
            if stdout {
                if let Some(us) = parse_time(line, "out_time_us=") {
                    progress(
                        (us / 1_000_000.0 / media.duration).clamp(0.0, 1.0) * 0.8,
                        "Finding silence…",
                    );
                }
            } else {
                if let Some(t) = parse_time(line, "silence_start: ") {
                    start = Some(t.max(0.0));
                }
                if let Some(t) = parse_time(line, "silence_end: ") {
                    if let Some(s) = start.take() {
                        silences.push(Span {
                            start: s,
                            end: t.min(media.duration),
                        });
                    }
                }
            }
        })?;
        if let Some(s) = start {
            silences.push(Span {
                start: s,
                end: media.duration,
            });
        }
        silences
    };
    silences.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut cuts = cuts_from_silence(&silences, media.duration, settings.padding);
    // A wholly silent file must never become an invalid empty video.
    if invert(&cuts, media.duration).is_empty() {
        return Err(
            "The entire video is below the silence threshold. Lower the threshold and try again."
                .into(),
        );
    }
    if settings.quality == 0 && !cuts.is_empty() {
        progress(0.8, "Locating keyframes…");
        let keys = keyframes(tools, media, cancel)?;
        cuts = align_to_keys(&cuts, &keys, media.duration);
    }
    let keep = invert(&cuts, media.duration);
    let duration = keep.iter().map(Span::len).sum();
    progress(1.0, "Ready to export");
    Ok(Analysis {
        keep,
        removed: cuts,
        duration,
        settings: settings.clone(),
        detected: silences.len(),
    })
}

pub fn keyframes(tools: &Tools, media: &Media, cancel: &Cancel) -> Result<Vec<f64>> {
    let mut argv = args(&[
        "-v",
        "error",
        "-select_streams",
        "V:0",
        "-show_packets",
        "-show_entries",
        "packet=pts_time,flags",
        "-of",
        "csv=p=0",
    ]);
    argv.push(media.path.as_os_str().into());
    let mut keys = vec![0.0];
    run(&tools.ffprobe, &argv, cancel, |out, l| {
        if out && l.contains('K') {
            if let Some(t) = l.split(',').next().and_then(|v| v.parse::<f64>().ok()) {
                if t.is_finite() {
                    keys.push((t - media.start_time).max(0.));
                }
            }
        }
    })?;
    keys.push(media.duration);
    keys.sort_by(f64::total_cmp);
    keys.dedup_by(|a, b| (*a - *b).abs() < 0.0001);
    Ok(keys)
}

fn copy_signature(tools: &Tools, media: &Media, cancel: &Cancel) -> Result<Value> {
    let mut a = args(&[
        "-v",
        "error",
        "-show_streams",
        "-show_data_hash",
        "sha256",
        "-of",
        "json",
    ]);
    a.push(media.path.as_os_str().into());
    let mut s = String::new();
    run(&tools.ffprobe, &a, cancel, |out, l| {
        if out {
            s.push_str(l);
        }
    })?;
    let v: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    let mut streams = vec![];
    for x in v["streams"].as_array().ok_or("Missing media information")? {
        if x["codec_type"] != "video" && x["codec_type"] != "audio" {
            continue;
        }
        let mut m = serde_json::Map::new();
        for k in [
            "codec_type",
            "codec_name",
            "profile",
            "width",
            "height",
            "pix_fmt",
            "sample_rate",
            "channels",
            "channel_layout",
            "time_base",
            "extradata_hash",
        ] {
            m.insert(k.into(), x[k].clone());
        }
        streams.push(Value::Object(m));
    }
    Ok(Value::Array(streams))
}

fn nvenc_available(tools: &Tools, lossless: bool, cancel: &Cancel) -> Result<bool> {
    let mut a = args(&[
        "-v",
        "error",
        "-nostdin",
        "-f",
        "lavfi",
        "-i",
        "color=size=128x128:duration=0.04",
        "-frames:v",
        "1",
        "-c:v",
        "h264_nvenc",
        "-preset",
        "p4",
    ]);
    if lossless {
        a.extend(args(&["-tune", "lossless", "-rc", "constqp", "-qp", "0"]));
    }
    a.extend(args(&["-f", "null", "-"]));
    let ok = run(&tools.ffmpeg, &a, cancel, |_, _| {}).is_ok();
    cancelled(cancel)?;
    Ok(ok)
}

/// Render a single video track with linked audio, using one pass when compatible.
pub fn export_timeline(
    tools: &Tools,
    timeline: &crate::timeline::Timeline,
    settings: &Settings,
    remove_silence: bool,
    output: &Path,
    cancel: &Cancel,
    mut progress: impl FnMut(f64, &str),
) -> Result<Exported> {
    settings.validate()?;
    if timeline.clips.is_empty() {
        return Err("Import a video first.".into());
    }
    let output = std::path::absolute(output).map_err(|e| e.to_string())?;
    if output.exists() {
        return Err("That output already exists. Choose a new name.".into());
    }
    if output
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case(settings.extension()))
    {
        return Err(format!("Choose a .{} filename.", settings.extension()));
    }
    let temp = TempDir::new(output.parent().ok_or("Choose an output folder.")?)?;
    let first = &timeline.clips[0].media;
    if settings.quality == 0 {
        let signature = copy_signature(tools, first, cancel)?;
        let mut seen = std::collections::HashSet::new();
        seen.insert(first.path.clone());
        for c in &timeline.clips {
            if seen.insert(c.media.path.clone())
                && copy_signature(tools, &c.media, cancel)? != signature
            {
                return Err("Original mode needs matching video and audio formats. Choose High quality to join these videos.".into());
            }
        }
    }
    if settings.quality == 1
        && timeline.clips.iter().any(|c| {
            c.media.pixel_format != first.pixel_format || c.media.audio_specs != first.audio_specs
        })
    {
        return Err("Lossless mode needs matching pixel formats and audio layouts. Choose High quality for mixed formats.".into());
    }
    let mut plans = Vec::new();
    let mut cache: std::collections::HashMap<PathBuf, Analysis> = std::collections::HashMap::new();
    let mut keys_cache = std::collections::HashMap::new();
    for (i, c) in timeline.clips.iter().enumerate() {
        cancelled(cancel)?;
        progress(
            i as f64 / timeline.clips.len() as f64 * 0.18,
            "Preparing clips…",
        );
        let mut start = c.start;
        let mut end = c.end;
        if settings.quality == 0 && (start > 0.001 || end < c.media.duration - 0.001) {
            if !keys_cache.contains_key(&c.media.path) {
                keys_cache.insert(c.media.path.clone(), keyframes(tools, &c.media, cancel)?);
            }
            let keys = &keys_cache[&c.media.path];
            let snap = |t: f64| {
                keys.iter()
                    .min_by(|a, b| (*a - t).abs().total_cmp(&(*b - t).abs()))
                    .copied()
                    .unwrap_or(t)
            };
            start = snap(start);
            end = snap(end);
            if end - start < 0.001 {
                return Err("A clip is shorter than its keyframe interval. Choose Lossless or High quality for exact cuts.".into());
            }
        }
        let spans = if remove_silence && c.media.audio_count > 0 {
            if !cache.contains_key(&c.media.path) {
                let a = match analyze(tools, &c.media, settings, cancel, |_, s| {
                    progress(i as f64 / timeline.clips.len() as f64 * 0.18, s)
                }) {
                    Ok(a) => a,
                    Err(e) if e.starts_with("The entire video is below") => Analysis {
                        keep: vec![],
                        removed: vec![Span {
                            start: 0.,
                            end: c.media.duration,
                        }],
                        duration: 0.,
                        settings: settings.clone(),
                        detected: 1,
                    },
                    Err(e) => return Err(e),
                };
                cache.insert(c.media.path.clone(), a);
            }
            cache[&c.media.path]
                .keep
                .iter()
                .filter_map(|s| {
                    let a = s.start.max(start);
                    let b = s.end.min(end);
                    if b - a > 0.001 {
                        Some(Span { start: a, end: b })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        } else {
            vec![Span { start, end }]
        };
        for span in spans {
            plans.push((c.media.clone(), span));
        }
    }
    if plans.is_empty() {
        return Err("No content remains with these silence settings.".into());
    }
    // Undoing an edit or splitting without removing content should not create extra encodes.
    let mut merged: Vec<(Arc<Media>, Span)> = vec![];
    for (m, s) in plans {
        if let Some((last_m, last_s)) = merged.last_mut() {
            if last_m.path == m.path && (last_s.end - s.start).abs() < 0.00001 {
                last_s.end = s.end;
                continue;
            }
        }
        merged.push((m, s));
    }
    let plans = merged;
    let total: f64 = plans.iter().map(|(_, s)| s.len()).sum();
    let mut done = 0.;
    let width = timeline
        .clips
        .iter()
        .map(|c| c.media.width)
        .max()
        .unwrap_or(1920)
        .div_ceil(2)
        * 2;
    let height = timeline
        .clips
        .iter()
        .map(|c| c.media.height)
        .max()
        .unwrap_or(1080)
        .div_ceil(2)
        * 2;
    let tracks = timeline
        .clips
        .iter()
        .map(|c| c.media.audio_count)
        .max()
        .unwrap_or(0);
    let high_depth = timeline.clips.iter().any(|c| {
        ["10", "12", "16"]
            .iter()
            .any(|bits| c.media.pixel_format.contains(bits))
    });
    progress(0.18, "Checking GPU acceleration…");
    let allow_gpu = settings.gpu
        && std::env::var_os("QUIETCUT_DISABLE_GPU").is_none()
        && settings.quality > 0
        && !high_depth;
    let gpu = allow_gpu
        && (settings.quality != 1 || first.pixel_format == "yuv420p")
        && nvenc_available(tools, settings.quality == 1, cancel)?;
    let mut cuda_filters = String::new();
    if gpu {
        let _ = run(
            &tools.ffmpeg,
            &args(&["-hide_banner", "-filters"]),
            cancel,
            |out, l| {
                if out {
                    cuda_filters.push_str(l);
                }
            },
        );
    }
    if std::env::var_os("QUIETCUT_FORCE_LEGACY").is_none() {
        match fast_export::try_export(
            tools,
            timeline,
            settings,
            remove_silence,
            &plans,
            &output,
            &temp,
            gpu,
            &cuda_filters,
            cancel,
            &mut progress,
        ) {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => {}
            Err(e) => {
                cancelled(cancel)?;
                if std::env::var_os("QUIETCUT_PROFILE").is_some() {
                    eprintln!("Single-pass fallback: {e}");
                }
                progress(0.18, "Using compatible export…");
            }
        }
    }
    let mut list = String::from("ffconcat version 1.0\n");
    for (i, (m, s)) in plans.iter().enumerate() {
        let piece = temp.0.join(format!("piece-{i:06}.mkv"));
        let label = format!(
            "{} · clip {} / {}…",
            if gpu {
                "NVIDIA GPU"
            } else if settings.quality == 0 {
                "Stream copy"
            } else {
                "CPU"
            },
            i + 1,
            plans.len()
        );
        if settings.quality == 0
            && !settings.level
            && s.start <= 0.001
            && s.end >= m.duration - 0.001
        {
            list.push_str(&format!("file '{}'\n", escape_concat(&m.path)?));
            done += s.len();
            continue;
        }
        if settings.quality == 0 {
            let cuts = invert(&[*s], m.duration);
            let a = Analysis {
                keep: vec![*s],
                removed: cuts,
                duration: s.len(),
                settings: settings.clone(),
                detected: 0,
            };
            let mut copy = settings.clone();
            copy.level = settings.level;
            export(tools, m, &a, &copy, &piece, cancel, |p, _| {
                progress(0.18 + (done + p * s.len()) / total * 0.70, &label)
            })?;
        } else {
            let cuda = gpu
                && cuda_filters.contains("pad_cuda")
                && ["h264", "hevc", "mpeg2video", "vc1", "vp9"].contains(&m.video_codec.as_str())
                && ["yuv420p", "nv12"].contains(&m.pixel_format.as_str())
                && m.width % 2 == 0
                && m.height % 2 == 0;
            let mut a = base_ffmpeg();
            if cuda {
                a.extend(args(&[
                    "-hwaccel",
                    "cuda",
                    "-hwaccel_output_format",
                    "cuda",
                ]));
            }
            arg(&mut a, "-ss", format!("{:.9}", s.start));
            arg(&mut a, "-i", m.path.as_os_str());
            if settings.quality >= 2 && m.audio_count < tracks {
                a.extend(args(&["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"]));
            }
            arg(&mut a, "-t", format!("{:.9}", s.len()));
            a.extend(args(&["-map", "0:V:0"]));
            for track in 0..tracks {
                arg(
                    &mut a,
                    "-map",
                    if track < m.audio_count {
                        format!("0:a:{track}")
                    } else {
                        "1:a:0".into()
                    },
                );
            }
            a.extend(args(&[
                "-map_metadata",
                "0",
                "-map_chapters",
                "-1",
                "-sn",
                "-dn",
            ]));
            // Keep original pixel dimensions; pad smaller sources rather than stretching them.
            let filter = if cuda {
                format!(
                    "scale_cuda=format=nv12,pad_cuda={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1"
                )
            } else if settings.quality == 1 {
                format!("pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1")
            } else {
                format!(
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,format={}",
                    if high_depth { "yuv420p10le" } else { "yuv420p" }
                )
            };
            arg(&mut a, "-vf", filter);
            if gpu {
                a.extend(args(&["-c:v", "h264_nvenc", "-preset", "p4"]));
                if settings.quality == 1 {
                    a.extend(args(&["-tune", "lossless", "-rc", "constqp", "-qp", "0"]));
                } else {
                    a.extend(args(&[
                        "-tune",
                        "hq",
                        "-rc",
                        "vbr",
                        "-cq",
                        if settings.quality == 2 { "18" } else { "23" },
                        "-b:v",
                        "0",
                        "-spatial-aq",
                        "1",
                    ]));
                }
            } else if settings.quality == 1 {
                a.extend(args(&["-c:v", "ffv1", "-level", "3", "-slicecrc", "1"]));
            } else {
                a.extend(args(&[
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    if settings.quality == 2 { "18" } else { "23" },
                ]));
            }
            for (flag, value) in &m.color {
                arg(&mut a, flag, value);
            }
            if settings.level {
                arg(&mut a, "-af", LEVEL_FILTER);
            }
            a.extend(args(&["-c:a", "pcm_f64le"]));
            if settings.quality >= 2 {
                a.extend(args(&["-ar", "48000", "-ac", "2"]));
            }
            a.extend(args(&[
                "-fps_mode",
                "passthrough",
                "-avoid_negative_ts",
                "disabled",
                "-progress",
                "pipe:1",
            ]));
            a.push(piece.as_os_str().into());
            if let Err(e) = progress_run(tools, &a, cancel, s.len(), |p| {
                progress(0.18 + (done + p * s.len()) / total * 0.70, &label)
            }) {
                cancelled(cancel)?;
                // A decoder/driver can reject a particular profile even after the encoder
                // capability test. Restart the whole export consistently with CPU encoding.
                if gpu {
                    let mut fallback = settings.clone();
                    fallback.gpu = false;
                    drop(temp);
                    progress(0., "GPU fallback · using CPU…");
                    return export_timeline(
                        tools,
                        timeline,
                        &fallback,
                        remove_silence,
                        &output,
                        cancel,
                        progress,
                    );
                } else {
                    return Err(e);
                }
            }
        }
        list.push_str(&format!("file '{}'\n", escape_concat(&piece)?));
        if settings.quality == 0 {
            list.push_str(&format!("duration {:.9}\n", s.len()));
        }
        done += s.len();
    }
    let list_path = temp.0.join("join.ffconcat");
    fs::write(&list_path, list).map_err(|e| e.to_string())?;
    let staging = temp.0.join(format!("finished.{}", settings.extension()));
    let mut a = base_ffmpeg();
    a.extend(args(&["-f", "concat", "-safe", "0", "-i"]));
    a.push(list_path.into_os_string());
    stream_maps(&mut a);
    a.extend(args(&["-c:v", "copy"]));
    if settings.quality < 2 {
        a.extend(args(&["-c:a", "copy"]));
    } else if settings.quality >= 2 {
        a.extend(args(&[
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ]));
    } else {
        a.extend(args(&["-c:a", "pcm_f64le"]));
    }
    a.extend(args(&[
        "-avoid_negative_ts",
        "disabled",
        "-progress",
        "pipe:1",
    ]));
    a.push(staging.as_os_str().into());
    progress_run(tools, &a, cancel, total, |p| {
        progress(
            0.88 + p * 0.1,
            if settings.level {
                "Evening audio…"
            } else {
                "Joining clips…"
            },
        )
    })?;
    cancelled(cancel)?;
    let info = probe(tools, &staging, cancel)?;
    cancelled(cancel)?;
    publish_output(&staging, &output)?;
    progress(1., "Saved");
    Ok(Exported {
        path: output,
        duration: info.duration,
        size: info.size,
    })
}

static JOB_COUNT: AtomicU64 = AtomicU64::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new(parent: &Path) -> Result<Self> {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let p = parent.join(format!(
            ".quietcut-{}-{id}-{}",
            std::process::id(),
            JOB_COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).map_err(|e| format!("Cannot create export workspace: {e}"))?;
        Ok(Self(p))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn base_ffmpeg() -> Vec<OsString> {
    args(&[
        "-hide_banner",
        "-nostdin",
        "-nostats",
        "-v",
        "warning",
        "-y",
    ])
}
fn stream_maps(argv: &mut Vec<OsString>) {
    argv.extend(args(&[
        "-map",
        "0:V:0",
        "-map",
        "0:a?",
        "-map_metadata",
        "0",
        "-map_chapters",
        "-1",
        "-sn",
        "-dn",
    ]));
}
fn progress_run(
    tools: &Tools,
    argv: &[OsString],
    cancel: &Cancel,
    duration: f64,
    mut progress: impl FnMut(f64),
) -> Result<()> {
    run(&tools.ffmpeg, argv, cancel, |out, l| {
        if out {
            if let Some(us) = parse_time(l, "out_time_us=") {
                progress((us / 1_000_000.0 / duration).clamp(0.0, 1.0));
            }
        }
    })
}

fn fast_progress_run(
    tools: &Tools,
    argv: &[OsString],
    cancel: &Cancel,
    duration: f64,
    frames: u64,
    mut progress: impl FnMut(f64),
) -> Result<()> {
    let mut encoded = EncoderProgress::default();
    let started = std::time::Instant::now();
    let mut first = true;
    run_guarded(
        &tools.ffmpeg,
        argv,
        cancel,
        Some(Duration::from_secs(20)),
        |out, line| {
            if out && encoded.advance(line) {
                if first {
                    first = false;
                    if std::env::var_os("QUIETCUT_PROFILE").is_some() {
                        eprintln!(
                            "First encoded progress: {:.3}s",
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
                // Muxed timestamps can lag behind encoding while audio is buffered.
                // Encoded frame count still gives real, monotonic render progress.
                progress(
                    (encoded.micros as f64 / 1_000_000. / duration)
                        .max(encoded.frames as f64 / frames as f64)
                        .clamp(0., 1.),
                );
            }
        },
    )
}

pub fn export(
    tools: &Tools,
    media: &Media,
    analysis: &Analysis,
    settings: &Settings,
    output: &Path,
    cancel: &Cancel,
    mut progress: impl FnMut(f64, &str),
) -> Result<Exported> {
    let absolute = std::path::absolute(output).map_err(|e| e.to_string())?;
    let output = absolute.as_path();
    settings.validate()?;
    cancelled(cancel)?;
    if !analysis.settings.detection_eq(settings) {
        return Err("Silence settings changed. Analyze again before exporting.".into());
    }
    if output.exists() {
        return Err("That output file already exists. Choose a new filename.".into());
    }
    if output
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| !s.eq_ignore_ascii_case(settings.extension()))
        .unwrap_or(true)
    {
        return Err(format!(
            "Choose a .{} filename for this quality mode.",
            settings.extension()
        ));
    }
    let parent = output.parent().ok_or("Choose a valid output folder.")?;
    let temp = TempDir::new(parent)?;
    let staging = temp.0.join(format!("finished.{}", settings.extension()));
    let list = temp.0.join("join.ffconcat");
    let mut concat = String::from("ffconcat version 1.0\n");
    if settings.quality == 0 {
        if analysis.removed.is_empty() {
            // One remux; no temporary duplicate of the source.
            concat.push_str(&format!("file '{}'\n", escape_concat(&media.path)?));
        } else {
            // Segment only at the source's existing random access points. Compressed packets
            // are copied unchanged; no decoder or encoder is used in this path.
            let mut boundaries: Vec<f64> = analysis
                .removed
                .iter()
                .flat_map(|s| [s.start, s.end])
                .filter(|t| *t > 0.001 && *t < media.duration - 0.001)
                .collect();
            boundaries.sort_by(f64::total_cmp);
            boundaries.dedup_by(|a, b| (*a - *b).abs() < 0.0001);
            let mut argv = base_ffmpeg();
            arg(&mut argv, "-i", media.path.as_os_str());
            stream_maps(&mut argv);
            argv.extend(args(&[
                "-c",
                "copy",
                "-avoid_negative_ts",
                "disabled",
                "-f",
                "segment",
                "-segment_format",
                "matroska",
                "-reset_timestamps",
                "1",
                "-segment_time_delta",
                "0.001",
                "-segment_times",
            ]));
            argv.push(
                boundaries
                    .iter()
                    .map(|t| format!("{t:.6}"))
                    .collect::<Vec<_>>()
                    .join(",")
                    .into(),
            );
            argv.extend(args(&["-progress", "pipe:1"]));
            argv.push(temp.0.join("part-%06d.mkv").into_os_string());
            progress(0.0, "Copying original streams…");
            progress_run(tools, &argv, cancel, media.duration, |p| {
                progress(p * 0.65, "Copying original streams…")
            })?;
            let mut edges = vec![0.0];
            edges.extend(boundaries);
            edges.push(media.duration);
            for (i, w) in edges.windows(2).enumerate() {
                let middle = (w[0] + w[1]) / 2.0;
                if !analysis
                    .removed
                    .iter()
                    .any(|s| middle >= s.start && middle < s.end)
                {
                    let part = temp.0.join(format!("part-{i:06}.mkv"));
                    if !part.exists() {
                        return Err("The source cannot be cut safely at its keyframes. Choose Lossless or High quality.".into());
                    }
                    concat.push_str(&format!(
                        "file '{}'\nduration {:.9}\n",
                        escape_concat(&part)?,
                        w[1] - w[0]
                    ));
                }
            }
        }
    } else {
        // Bounded-memory, seek-based encoding: decode only retained spans, with no giant
        // split graph, no pixel copies through Rust, and no silence video to encode.
        let mut done = 0.0;
        for (i, span) in analysis.keep.iter().enumerate() {
            cancelled(cancel)?;
            let part = temp.0.join(format!("part-{i:06}.mkv"));
            let mut argv = base_ffmpeg();
            arg(&mut argv, "-ss", format!("{:.9}", span.start));
            arg(&mut argv, "-i", media.path.as_os_str());
            arg(&mut argv, "-t", format!("{:.9}", span.len()));
            stream_maps(&mut argv);
            if settings.quality == 1 {
                argv.extend(args(&["-c:v", "ffv1", "-level", "3", "-slicecrc", "1"]));
            } else {
                argv.extend(args(&[
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    if settings.quality == 2 { "18" } else { "23" },
                ]));
                // Keep the source bit depth and chroma format where x264 supports it.
                // No forced yuv420p conversion, scaling, or frame-rate conversion.
            }
            for (flag, value) in &media.color {
                arg(&mut argv, flag, value);
            }
            // f64 can represent the decoded integer/float samples exactly. Avoid one AAC
            // encoder delay per cut; encode audio once, continuously, after the join.
            if settings.level {
                arg(&mut argv, "-af", LEVEL_FILTER);
            }
            argv.extend(args(&[
                "-c:a",
                "pcm_f64le",
                "-fps_mode",
                "passthrough",
                "-progress",
                "pipe:1",
            ]));
            argv.push(part.as_os_str().into());
            let label = format!("Cutting {} / {}…", i + 1, analysis.keep.len());
            progress_run(tools, &argv, cancel, span.len(), |p| {
                progress((done + p * span.len()) / analysis.duration * 0.85, &label)
            })?;
            concat.push_str(&format!("file '{}'\n", escape_concat(&part)?));
            done += span.len();
        }
    }
    fs::write(&list, concat).map_err(|e| e.to_string())?;
    let mut argv = base_ffmpeg();
    argv.extend(args(&["-f", "concat", "-safe", "0", "-i"]));
    argv.push(list.into_os_string());
    stream_maps(&mut argv);
    argv.extend(args(&["-c:v", "copy"]));
    if settings.level && settings.quality == 0 {
        arg(&mut argv, "-af", LEVEL_FILTER);
    }
    if settings.quality == 0 && !settings.level {
        argv.extend(args(&["-c:a", "copy"]));
    } else if settings.quality >= 2 {
        argv.extend(args(&[
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ]));
    } else {
        // Preserve decoded samples exactly when volume has not been intentionally changed.
        argv.extend(args(&["-c:a", "pcm_f64le"]));
    }
    argv.extend(args(&[
        "-avoid_negative_ts",
        "disabled",
        "-progress",
        "pipe:1",
    ]));
    argv.push(staging.as_os_str().into());
    let base = if settings.quality == 0 {
        if analysis.removed.is_empty() {
            0.0
        } else {
            0.65
        }
    } else {
        0.85
    };
    let label = if settings.level {
        "Evening audio volume…"
    } else {
        "Finishing video…"
    };
    progress_run(tools, &argv, cancel, analysis.duration, |p| {
        progress(base + p * (0.98 - base), label)
    })?;
    cancelled(cancel)?;
    progress(0.99, "Checking output…");
    let info = probe(tools, &staging, cancel)?;
    if info.size == 0 || info.duration <= 0.0 {
        return Err("The media engine produced an empty output.".into());
    }
    cancelled(cancel)?;
    publish_output(&staging, output)?;
    progress(1.0, "Done");
    Ok(Exported {
        path: output.to_owned(),
        duration: info.duration,
        size: info.size,
    })
}

fn publish_output(staging: &Path, output: &Path) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    let result = fs::rename(staging, output);
    #[cfg(target_os = "linux")]
    let result = {
        use std::{
            ffi::{CString, c_char, c_int, c_uint},
            os::unix::ffi::OsStrExt,
        };

        unsafe extern "C" {
            fn renameat2(
                from_fd: c_int,
                from: *const c_char,
                to_fd: c_int,
                to: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }

        const AT_FDCWD: c_int = -100;
        const RENAME_NOREPLACE: c_uint = 1;
        let from = CString::new(staging.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let to = CString::new(output.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        // Both paths are NUL-terminated and remain alive through the syscall.
        let status = unsafe {
            renameat2(
                AT_FDCWD,
                from.as_ptr(),
                AT_FDCWD,
                to.as_ptr(),
                RENAME_NOREPLACE,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    };
    result.map_err(|e| format!("Cannot save output: {e}"))
}

fn escape_concat(path: &Path) -> Result<String> {
    let p = path
        .to_str()
        .ok_or("This filename cannot be represented in the media engine.")?;
    #[cfg(windows)]
    let p = p.replace('\\', "/");
    if p.contains(['\n', '\r']) {
        return Err("Filenames with line breaks are not supported.".into());
    }
    Ok(p.replace('\'', "'\\''"))
}

pub fn human_size(size: u64) -> String {
    let n = size as f64;
    if n >= 1_073_741_824.0 {
        format!("{:.2} GB", n / 1_073_741_824.0)
    } else if n >= 1_048_576.0 {
        format!("{:.1} MB", n / 1_048_576.0)
    } else {
        format!("{:.0} KB", n / 1024.0)
    }
}
pub fn human_time(seconds: f64) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    if ms >= 3_600_000 {
        format!(
            "{}:{:02}:{:02}.{:01}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms / 100 % 10
        )
    } else {
        format!("{}:{:02}.{:01}", ms / 60_000, ms / 1000 % 60, ms / 100 % 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padding_protects_speech_edges() {
        let cuts = cuts_from_silence(
            &[
                Span {
                    start: 0.0,
                    end: 2.0,
                },
                Span {
                    start: 4.0,
                    end: 6.0,
                },
                Span {
                    start: 9.0,
                    end: 10.0,
                },
            ],
            10.0,
            0.1,
        );
        assert_eq!(
            cuts,
            vec![
                Span {
                    start: 0.0,
                    end: 1.9
                },
                Span {
                    start: 4.1,
                    end: 5.9
                },
                Span {
                    start: 9.1,
                    end: 10.0
                }
            ]
        );
        let keep = invert(&cuts, 10.0);
        assert!((keep.iter().map(Span::len).sum::<f64>() - 5.4).abs() < 0.0001);
    }
    #[test]
    fn copy_never_removes_sound_outside_silence() {
        let c = vec![
            Span {
                start: 1.1,
                end: 6.9,
            },
            Span {
                start: 8.1,
                end: 9.9,
            },
        ];
        assert_eq!(
            align_to_keys(&c, &[0., 2., 4., 6., 8., 10.], 12.),
            vec![Span { start: 2., end: 6. }]
        );
    }
    #[test]
    fn bad_settings_are_rejected() {
        let mut s = Settings::default();
        s.padding = f64::NAN;
        assert!(s.validate().is_err());
        s.padding = 0.08;
        s.threshold = 0.0;
        assert!(s.validate().is_err());
    }
    #[test]
    fn silence_uses_exact_cuts_and_presets_keep_the_same_speech_floor() {
        let mut s = Settings::default();
        s.exact_silence(false);
        assert_eq!(s.quality, 0);
        s.exact_silence(true);
        assert_eq!(s.quality, 2);
        s.quality = 1;
        s.exact_silence(true);
        assert_eq!(s.quality, 1);
        s.silence_preset(1);
        let balanced = s.clone();
        s.silence_preset(2);
        assert_eq!(s.threshold, balanced.threshold);
        assert!(s.min_silence < balanced.min_silence);
        assert!(s.padding < balanced.padding);
        s.silence_preset(3);
        assert!(!s.adaptive);
    }
    #[test]
    fn previous_preferences_keep_their_values_and_receive_new_defaults() {
        let s: Settings = serde_json::from_str(
            r#"{"threshold":-35,"min_silence":0.5,"padding":0.08,"quality":2,"level":true,"dark":true}"#,
        ).unwrap();
        assert!(s.adaptive && s.ask_save && s.gpu && s.level);
        assert_eq!(s.output_dir, None);
        assert_eq!(s.quality, 2);
        assert!(s.validate().is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn quoted_unicode_paths_are_safe() {
        assert_eq!(
            escape_concat(Path::new("C:\\视频\\it's.mkv")).unwrap(),
            "C:/视频/it'\\''s.mkv"
        );
    }

    #[test]
    fn encoder_heartbeat_requires_real_monotonic_progress() {
        let mut progress = EncoderProgress::default();
        assert!(!progress.advance("frame=0"));
        assert!(!progress.advance("out_time_us=N/A"));
        assert!(!progress.advance("out_time_us=-1234"));
        assert!(progress.advance("frame= 12"));
        assert!(!progress.advance("frame=12"));
        assert!(!progress.advance("frame=11"));
        assert!(progress.advance("out_time_us=400000"));
        assert!(!progress.advance("progress=continue"));
        assert!(!progress.advance("out_time_us=400000"));
    }

    #[test]
    fn stalled_child_is_stopped_without_cancelling_the_fallback() {
        #[cfg(windows)]
        let (program, arguments) = (
            PathBuf::from(std::env::var_os("WINDIR").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            args(&[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.WriteLine('frame=0'); Start-Sleep -Seconds 5",
            ]),
        );
        #[cfg(not(windows))]
        let (program, arguments) = (
            PathBuf::from("/bin/sh"),
            args(&["-c", "echo frame=0; exec sleep 5"]),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let started = std::time::Instant::now();
        let result = run_guarded(
            &program,
            &arguments,
            &cancel,
            Some(Duration::from_millis(150)),
            |_, _| {},
        );
        assert!(result.unwrap_err().contains("stopped making progress"));
        assert!(!cancel.load(Ordering::Relaxed));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
