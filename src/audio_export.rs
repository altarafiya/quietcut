use super::*;
use crate::{plan, timeline::Timeline};

const RATE: f64 = 48_000.;

pub fn export_audio(
    tools: &Tools,
    timeline: &Timeline,
    settings: &Settings,
    remove_silence: bool,
    output: &Path,
    cancel: &Cancel,
    mut progress: impl FnMut(f64, &str),
) -> Result<Exported> {
    let mut settings = settings.clone();
    settings.quality = 2;
    settings.validate()?;
    if !timeline.clips.iter().any(|clip| clip.media.audio_count > 0) {
        return Err("The timeline has no audio tracks to export.".into());
    }
    let output = export_destination(output, "m4a")?;
    let temp = TempDir::new(output.parent().ok_or("Choose an output folder.")?)?;
    let plan = plan::prepare(
        tools,
        timeline,
        &settings,
        remove_silence,
        cancel,
        |p, status| progress(p * 0.18, status),
    )?;
    let tracks = plan
        .spans
        .iter()
        .map(|span| span.media.audio_count)
        .max()
        .unwrap_or(0);
    if tracks == 0 {
        return Err("No audio remains with these silence settings.".into());
    }

    // One decode per clip, regardless of the number of removed pauses. When
    // clips differ, join lossless audio pieces before the single AAC encode.
    let groups: Vec<_> = plan.spans.chunk_by(|a, b| a.clip == b.clip).collect();
    let single = groups.len() == 1;
    let staging = temp.0.join("finished.m4a");
    let graph_path = temp.0.join("audio.ffgraph");
    let mut list = String::from("ffconcat version 1.0\n");
    let mut done = 0.;
    for (i, spans) in groups.into_iter().enumerate() {
        cancelled(cancel)?;
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for span in spans {
            let start = (span.start * RATE).round() as u64;
            let end = (span.end * RATE).round() as u64;
            if start < end {
                if let Some(last) = ranges.last_mut() {
                    if last.1 >= start {
                        last.1 = last.1.max(end);
                        continue;
                    }
                }
                ranges.push((start, end));
            }
        }
        let samples: u64 = ranges.iter().map(|(start, end)| end - start).sum();
        let last = ranges.last().ok_or("The audio selection is too short.")?.1;
        let media = &spans[0].media;
        let mut graph = String::new();
        let level = if settings.level {
            format!(",{LEVEL_FILTER}")
        } else {
            String::new()
        };
        for track in 0..tracks {
            if track > 0 {
                graph.push(';');
            }
            if track < media.audio_count {
                let input = format!(
                    "[0:a:{track}]aresample=48000:async=1:first_pts=0,aformat=channel_layouts=stereo,apad,atrim=end_sample={last},asetpts=N/SR/TB{level}"
                );
                append_audio_cuts(&mut graph, &input, track, &ranges);
            } else {
                graph.push_str(&format!(
                    "anullsrc=r=48000:cl=stereo,atrim=end_sample={samples},asetpts=N/SR/TB[a{track}]"
                ));
            }
        }
        fs::write(&graph_path, graph).map_err(|e| e.to_string())?;
        let mut argv = base_ffmpeg();
        if media.audio_count > 0 {
            argv.push("-vn".into());
            arg(&mut argv, "-i", media.path.as_os_str());
        }
        arg(&mut argv, "-/filter_complex", graph_path.as_os_str());
        for track in 0..tracks {
            arg(&mut argv, "-map", format!("[a{track}]"));
        }
        let piece = if single {
            argv.extend(args(&[
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
            ]));
            staging.clone()
        } else {
            argv.extend(args(&["-c:a", "pcm_s24le"]));
            temp.0.join(format!("audio-{i:06}.mka"))
        };
        argv.extend(args(&["-progress", "pipe:1"]));
        argv.push(piece.as_os_str().into());
        let seconds = samples as f64 / RATE;
        let weight = if single { 0.79 } else { 0.60 };
        progress_run(tools, &argv, cancel, seconds, |p| {
            progress(
                0.18 + weight * (done + p * seconds) / plan.duration,
                "Exporting audio…",
            )
        })?;
        if !single {
            list.push_str(&format!(
                "file '{}'\nduration {seconds:.9}\n",
                escape_concat(&piece)?
            ));
        }
        done += seconds;
    }
    if !single {
        let list_path = temp.0.join("audio.ffconcat");
        fs::write(&list_path, list).map_err(|e| e.to_string())?;
        let mut argv = base_ffmpeg();
        argv.extend(args(&["-xerror", "-f", "concat", "-safe", "0", "-i"]));
        argv.push(list_path.into_os_string());
        argv.extend(args(&[
            "-map",
            "0:a",
            "-af",
            "asetpts=N/SR/TB",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            "-progress",
            "pipe:1",
        ]));
        argv.push(staging.as_os_str().into());
        progress_run(tools, &argv, cancel, done, |p| {
            progress(0.78 + 0.19 * p, "Joining audio…")
        })?;
    }

    cancelled(cancel)?;
    let info = probe_file(tools, &staging, cancel)?;
    let streams = info["streams"]
        .as_array()
        .ok_or("Cannot read the exported audio.")?;
    if streams.len() != tracks || streams.iter().any(|stream| stream["codec_type"] != "audio") {
        return Err("The export did not contain the expected audio tracks.".into());
    }
    let duration = number(info["format"].get("duration"))
        .filter(|duration| duration.is_finite() && *duration > 0.)
        .ok_or("The exported audio has no valid duration.")?;
    if (duration - done).abs() >= 0.1 {
        return Err(format!(
            "Expected {done:.3} s of audio; got {duration:.3} s."
        ));
    }
    let size = fs::metadata(&staging).map_err(|e| e.to_string())?.len();
    cancelled(cancel)?;
    publish_output(&staging, &output)?;
    progress(1., "Saved");
    Ok(Exported {
        path: output,
        duration,
        size,
    })
}
