//! One decoder/encoder session for ordered CFR cuts. Unsupported timelines use
//! the existing segmented renderer. Validation never changes the input media.
use super::*;

#[path = "timestamp_export.rs"]
mod timestamp;

struct RenderPlan {
    graph: String,
    duration: f64,
    input_duration: f64,
    time_base: u64,
    frames: u64,
    ranges: usize,
    copy_timestamps: bool,
}

#[derive(Debug)]
struct Grid {
    num: u64,
    den: u64,
    frames: u64,
}

impl Grid {
    fn frame(&self, seconds: f64) -> u64 {
        // Bounds are half-open: retain frames with source PTS >= start and < end.
        ((seconds * self.num as f64 / self.den as f64 - 0.000001)
            .ceil()
            .max(0.) as u64)
            .min(self.frames)
    }
    fn samples_per_frame(&self) -> Option<u64> {
        let samples = 48_000_u64.checked_mul(self.den)?;
        (samples % self.num == 0).then_some(samples / self.num)
    }
}

fn rational(value: &Value) -> Option<(u64, u64)> {
    let (num, den) = value.as_str()?.split_once('/')?;
    let (num, den) = (num.parse::<u64>().ok()?, den.parse::<u64>().ok()?);
    (num > 0 && den > 0).then_some((num, den))
}

/// An average rate alone cannot identify variable-rate video. Verify the packet
/// presentation times too: exactly one packet per grid point, including B frames.
fn validated_grid(tools: &Tools, m: &Media, cancel: &Cancel) -> Result<Option<Grid>> {
    let mut argv = args(&[
        "-v",
        "error",
        "-select_streams",
        "V:0",
        "-show_entries",
        "stream=r_frame_rate,avg_frame_rate,nb_frames,start_time",
        "-of",
        "json",
    ]);
    argv.push(m.path.as_os_str().into());
    let mut json = String::new();
    run(&tools.ffprobe, &argv, cancel, |out, line| {
        if out && json.len() < 64_000 {
            json.push_str(line);
        }
    })?;
    let value: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let s = &value["streams"][0];
    let Some((num, den)) = rational(&s["r_frame_rate"]) else {
        return Ok(None);
    };
    let Some((avg_num, avg_den)) = rational(&s["avg_frame_rate"]) else {
        return Ok(None);
    };
    let frames = number(s.get("nb_frames")).unwrap_or(0.) as u64;
    // OBS can store exact 60 fps pictures in a 1/60 clock. Precision of the
    // container clock does not disprove CFR: the packet-grid check below does.
    if num.checked_mul(avg_den) != avg_num.checked_mul(den)
        || !(1..=10_000_000).contains(&frames)
        || number(s.get("start_time")).unwrap_or(1.).abs() > 0.000001
        || m.start_time.abs() > 0.000001
    {
        return Ok(None);
    }
    let grid = Grid { num, den, frames };
    if m.audio_count > 0 && grid.samples_per_frame().is_none() {
        return Ok(None);
    }

    let mut seen = vec![false; frames as usize];
    let mut valid = true;
    let mut count = 0_u64;
    let mut argv = args(&[
        "-v",
        "error",
        "-select_streams",
        "V:0",
        "-show_entries",
        "packet=pts_time",
        "-of",
        "csv=p=0",
    ]);
    argv.push(m.path.as_os_str().into());
    run(&tools.ffprobe, &argv, cancel, |out, line| {
        if !out || line.trim().is_empty() {
            return;
        }
        let Some(time) = line.split(',').next().and_then(|s| s.parse::<f64>().ok()) else {
            valid = false;
            return;
        };
        let frame = time * num as f64 / den as f64;
        let nearest = frame.round();
        if !frame.is_finite()
            || (frame - nearest).abs() > 0.001
            || nearest < 0.
            || nearest >= frames as f64
        {
            valid = false;
            return;
        }
        let slot = &mut seen[nearest as usize];
        if *slot {
            valid = false;
        }
        *slot = true;
        count += 1;
    })?;
    Ok((valid && count == frames).then_some(grid))
}

fn frame_ranges(grid: &Grid, plans: &[(Arc<Media>, Span)]) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = vec![];
    for (_, span) in plans {
        let (start, end) = (grid.frame(span.start), grid.frame(span.end));
        if end <= start {
            continue;
        }
        if let Some(last) = ranges.last_mut() {
            if last.1 >= start {
                last.1 = last.1.max(end);
                continue;
            }
        }
        ranges.push((start, end));
    }
    ranges
}

fn selection(ranges: &[(u64, u64)]) -> String {
    // FFmpeg rejects a long left-associated sum at its expression depth limit.
    // A balanced decision tree also reduces each frame's work to O(log cuts).
    match ranges {
        [] => "0".into(),
        [(a, b)] => format!("gte(n,{a})*lt(n,{b})"),
        _ => {
            let mid = ranges.len() / 2;
            let (a, b) = ranges[mid];
            format!(
                "if(lt(n,{a}),{},if(lt(n,{b}),1,{}))",
                selection(&ranges[..mid]),
                selection(&ranges[mid + 1..]),
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_export(
    tools: &Tools,
    timeline: &crate::timeline::Timeline,
    settings: &Settings,
    remove_silence: bool,
    plans: &[(Arc<Media>, Span)],
    output: &Path,
    temp: &TempDir,
    gpu: bool,
    cuda_filters: &str,
    cancel: &Cancel,
    progress: &mut impl FnMut(f64, &str),
) -> Result<Option<Exported>> {
    if settings.quality == 0 || plans.len() > 4000 {
        return Ok(None);
    }
    let m = &plans[0].0;
    // One pass can also handle ordered manual cuts from one source. Leveling
    // across arbitrary edits retains the per-clip reset behavior in the old path.
    if timeline.clips.iter().any(|clip| clip.media.path != m.path)
        || plans.iter().any(|(source, _)| source.path != m.path)
        || plans
            .windows(2)
            .any(|p| p[0].1.end > p[1].1.start + 0.000001)
        || (settings.level && timeline.clips.len() != 1)
        || (settings.level && plans.len() > 1 && !remove_silence)
        || (settings.quality == 1 && m.audio_specs.iter().any(|(rate, _)| *rate != 48000))
        || !["yuv420p", "nv12"].contains(&m.pixel_format.as_str())
        || !m.width.is_multiple_of(2)
        || !m.height.is_multiple_of(2)
    {
        return Ok(None);
    }
    let total: f64 = plans.iter().map(|(_, span)| span.len()).sum();
    // Seeking is preferable for very sparse selections or long discarded heads.
    if total < plans.last().unwrap().1.end * 0.4 || plans[0].1.start > 5. {
        return Ok(None);
    }
    progress(0.18, "Preparing fast export…");
    let cuda = gpu
        && cuda_filters.contains("scale_cuda")
        && ["h264", "hevc", "mpeg2video", "vc1", "vp9"].contains(&m.video_codec.as_str());
    let format = if cuda {
        "scale_cuda=format=nv12"
    } else {
        "format=yuv420p"
    };
    let plan = if let Some(grid) = validated_grid(tools, m, cancel)? {
        grid_plan(&grid, m, plans, settings.level, format)
    } else {
        timestamp::build(tools, m, plans, settings.level, format, cancel)?
    };
    let Some(plan) = plan else { return Ok(None) };
    render(
        tools, m, settings, &plan, output, temp, gpu, cuda, cancel, progress,
    )
}

fn grid_plan(
    grid: &Grid,
    m: &Media,
    plans: &[(Arc<Media>, Span)],
    level: bool,
    format: &str,
) -> Option<RenderPlan> {
    let ranges = frame_ranges(grid, plans);
    let end = ranges.last()?.1;
    let count: u64 = ranges.iter().map(|(a, b)| b - a).sum();
    let duration = count as f64 * grid.den as f64 / grid.num as f64;
    let expression = selection(&ranges);
    let mut graph = format!(
        "[0:V:0]select='{expression}',settb=expr=1/{},setpts=N*{},{format},setsar=1[v]",
        grid.num, grid.den,
    );
    for track in 0..m.audio_count {
        let samples = grid.samples_per_frame().unwrap();
        // Preserve source silence while running the causal compressor, so it
        // releases before the next retained phrase. No lookahead gain smoothing.
        // Identical audio/video frame indices avoid rounding drift at every cut.
        let level = if level {
            format!(",{LEVEL_FILTER}")
        } else {
            String::new()
        };
        graph.push_str(&format!(
            ";[0:a:{track}]aresample=48000:async=1:first_pts=0,apad,atrim=end_sample={},asetpts=PTS-STARTPTS{level},asetnsamples=n={samples}:p=0,aselect='{expression}',asetpts=N/SR/TB[a{track}]",
            end * samples,
        ));
    }
    Some(RenderPlan {
        graph,
        duration,
        input_duration: end as f64 * grid.den as f64 / grid.num as f64,
        time_base: grid.num,
        frames: count,
        ranges: ranges.len(),
        copy_timestamps: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn render(
    tools: &Tools,
    m: &Media,
    settings: &Settings,
    plan: &RenderPlan,
    output: &Path,
    temp: &TempDir,
    gpu: bool,
    cuda: bool,
    cancel: &Cancel,
    progress: &mut impl FnMut(f64, &str),
) -> Result<Option<Exported>> {
    // File-valued graph avoids Windows' command-line length limit with many cuts.
    let graph_path = temp.0.join("single-pass.ffgraph");
    fs::write(&graph_path, &plan.graph).map_err(|e| e.to_string())?;
    let staging = temp.0.join(format!("single-pass.{}", settings.extension()));
    let mut argv = base_ffmpeg();
    if plan.copy_timestamps {
        argv.push("-copyts".into());
    }
    if cuda {
        argv.extend(args(&[
            "-hwaccel",
            "cuda",
            "-hwaccel_output_format",
            "cuda",
        ]));
    }
    arg(&mut argv, "-t", format!("{:.9}", plan.input_duration));
    arg(&mut argv, "-i", m.path.as_os_str());
    arg(&mut argv, "-/filter_complex", graph_path.as_os_str());
    argv.extend(args(&["-map", "[v]"]));
    for track in 0..m.audio_count {
        arg(&mut argv, "-map", format!("[a{track}]"));
    }
    argv.extend(args(&[
        "-map_metadata",
        "0",
        "-map_chapters",
        "-1",
        "-sn",
        "-dn",
    ]));
    if gpu {
        argv.extend(args(&["-c:v", "h264_nvenc", "-preset", "p4"]));
        if settings.quality == 1 {
            argv.extend(args(&["-tune", "lossless", "-rc", "constqp", "-qp", "0"]));
        } else {
            argv.extend(args(&[
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
    }
    for (flag, value) in &m.color {
        arg(&mut argv, flag, value);
    }
    if settings.quality == 1 {
        argv.extend(args(&["-c:a", "pcm_f64le"]));
    } else {
        argv.extend(args(&[
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-movflags",
            "+faststart",
        ]));
    }
    arg(
        &mut argv,
        "-enc_time_base:v",
        format!("1:{}", plan.time_base),
    );
    argv.extend(args(&[
        "-fps_mode",
        "passthrough",
        "-avoid_negative_ts",
        "disabled",
        "-progress",
        "pipe:1",
    ]));
    argv.push(staging.as_os_str().into());
    let started = std::time::Instant::now();
    if std::env::var_os("QUIETCUT_PROFILE").is_some() {
        eprintln!(
            "Single pass: {} ranges, {} frames, {:.6}s, GPU={gpu}, CUDA={cuda}, timestamp-aware={}",
            plan.ranges, plan.frames, plan.duration, plan.copy_timestamps,
        );
    }
    fast_progress_run(tools, &argv, cancel, plan.duration, plan.frames, |p| {
        progress(
            0.18 + p * 0.80,
            if gpu {
                "NVIDIA GPU · fast export…"
            } else {
                "CPU · fast export…"
            },
        );
    })?;
    cancelled(cancel)?;
    let info = probe(tools, &staging, cancel)?;
    // Refuse a truncated or drifting render rather than publish it as success.
    if (info.duration - plan.duration).abs() > 0.08 {
        return Err(format!(
            "Duration mismatch: expected {:.6}s, got {:.6}s",
            plan.duration, info.duration
        ));
    }
    // A corrupted input or decoder can emit fewer pictures than packet metadata
    // promises. Verify before publishing, including Matroska without nb_frames.
    let mut verify = args(&[
        "-v",
        "error",
        "-select_streams",
        "V:0",
        "-count_packets",
        "-show_entries",
        "stream=nb_read_packets",
        "-of",
        "csv=p=0",
    ]);
    verify.push(staging.as_os_str().into());
    let mut actual_frames = None;
    run(&tools.ffprobe, &verify, cancel, |out, line| {
        if out {
            actual_frames = line.trim().parse::<u64>().ok();
        }
    })?;
    if actual_frames != Some(plan.frames)
        || info.audio_count != m.audio_count
        || info.width != m.width
        || info.height != m.height
    {
        return Err("Export frame or stream verification failed.".into());
    }
    cancelled(cancel)?;
    publish_output(&staging, output)?;
    if std::env::var_os("QUIETCUT_PROFILE").is_some() {
        eprintln!(
            "Single-pass encoding: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    progress(1., "Saved");
    Ok(Some(Exported {
        path: output.to_owned(),
        duration: info.duration,
        size: info.size,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_open_frame_boundaries_and_sample_grid() {
        let grid = Grid {
            num: 60,
            den: 1,
            frames: 41595,
        };
        assert_eq!(grid.frame(0.57), 35);
        assert_eq!(grid.frame(16.07), 965);
        assert_eq!(grid.frame(0.1 + 0.2), 18);
        assert_eq!(grid.frame(693.269333), 41595);
        assert_eq!(grid.samples_per_frame(), Some(800));
        assert_eq!(
            Grid {
                num: 30000,
                den: 1001,
                frames: 1
            }
            .samples_per_frame(),
            None
        );
    }

    #[test]
    fn thousands_of_cuts_stay_below_expression_depth_limit() {
        let ranges: Vec<_> = (0..4000).map(|n| (n * 10, n * 10 + 5)).collect();
        let mut depth = 0;
        let mut maximum = 0;
        for c in selection(&ranges).chars() {
            if c == '(' {
                depth += 1;
                maximum = maximum.max(depth);
            }
            if c == ')' {
                depth -= 1;
            }
        }
        assert_eq!(depth, 0);
        assert!(maximum <= 26, "Expression nesting: {maximum}");
    }
}
