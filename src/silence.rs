//! One audio decode, relative RMS silence detection, and native quiet-speech protection.
use crate::engine::*;
use std::{
    io::{BufReader, Read},
    process::Stdio,
    sync::atomic::Ordering,
};
use webrtc_vad::{SampleRate, Vad, VadMode};
struct Frame {
    rms: f32,
    voice: bool,
}
pub fn detect(
    tools: &Tools,
    media: &Media,
    settings: &Settings,
    cancel: &Cancel,
    mut progress: impl FnMut(f64, &str),
) -> Result<Vec<Span>> {
    let channels = media
        .audio_specs
        .iter()
        .map(|(_, n)| *n as usize)
        .sum::<usize>()
        .max(1);
    let inputs = (0..media.audio_count)
        .map(|i| format!("[0:a:{i}]"))
        .collect::<String>();
    let filter = if media.audio_count > 1 {
        format!(
            "{inputs}amerge=inputs={},aresample=16000:async=1:first_pts=0[a]",
            media.audio_count
        )
    } else {
        "[0:a:0]aresample=16000:async=1:first_pts=0[a]".into()
    };
    let mut child = media_command(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-vn", "-i"])
        .arg(&media.path)
        .args([
            "-filter_complex",
            &filter,
            "-map",
            "[a]",
            "-ar",
            "16000",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::with_capacity(131072, child.stdout.take().unwrap());
    let mut bytes = vec![0u8; 160 * channels * 4];
    let mut frames = Vec::new();
    let mut vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::VeryAggressive);
    let mut mono = [0i16; 160];
    let mut peaks = vec![0f32; channels];
    let mut powers = vec![0f32; channels];
    progress(0., "Finding silence · protecting quiet speech…");
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Cancelled".into());
        }
        if reader.read_exact(&mut bytes).is_err() {
            break;
        }
        peaks.fill(0.);
        powers.fill(0.);
        for (i, b) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let n = f32::from_le_bytes(*b).abs();
            peaks[i % channels] = peaks[i % channels].max(n);
            powers[i % channels] += n * n;
        }
        let best = (0..channels)
            .max_by(|a, b| powers[*a].total_cmp(&powers[*b]))
            .unwrap_or(0);
        let peak = peaks[best];
        let rms = (powers[best] / 160.).sqrt();
        // Gain is used only for VAD classification, never applied to output or thresholding.
        let gain = (0.10 / peak.max(1e-9)).clamp(1., 16.);
        for (i, sample) in mono.iter_mut().enumerate() {
            let k = (i * channels + best) * 4;
            let value = f32::from_le_bytes(bytes[k..k + 4].try_into().unwrap());
            *sample = (value * gain * 32767.).clamp(-32768., 32767.) as i16;
        }
        let voice = peak > 1e-8 && vad.is_voice_segment(&mono).unwrap_or(true);
        frames.push(Frame { rms, voice });
        if frames.len() % 200 == 0 {
            progress(
                (frames.len() as f64 * 0.01 / media.duration).min(1.),
                "Finding silence · protecting quiet speech…",
            );
        }
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err("Could not analyze the audio track.".into());
    }
    // Typical speech, rather than a single peak, sets the relative threshold.
    // Selecting quantiles is linear and reuses the already-decoded envelope.
    let mut levels: Vec<f32> = frames.iter().map(|f| f.rms).collect();
    if levels.is_empty() {
        return Ok(vec![]);
    }
    let index = levels.len() * 9 / 10;
    let reference = *levels.select_nth_unstable_by(index, f32::total_cmp).1;
    let index = levels.len() / 5;
    let noise = *levels.select_nth_unstable_by(index, f32::total_cmp).1;
    let margin = 22. + (-50. - settings.threshold).max(0.);
    let db = (20. * (reference.max(1e-10) as f64).log10() - margin).clamp(-110., -20.);
    let threshold = 10f64.powf(db / 20.) as f32;
    let voice_floor = (noise * 3.1623).max(reference * 0.00001);
    let mut protected = vec![false; frames.len()];
    for (i, f) in frames.iter().enumerate() {
        if f.voice && f.rms > voice_floor {
            for p in &mut protected[i.saturating_sub(4)..(i + 5).min(frames.len())] {
                *p = true;
            }
        }
    }
    let mut result = vec![];
    let mut start = None;
    for (i, f) in frames.iter().enumerate() {
        let quiet = f.rms < threshold && !protected[i];
        if quiet {
            if start.is_none() {
                start = Some(i as f64 * 0.01);
            }
        } else if let Some(a) = start.take() {
            let end = i as f64 * 0.01;
            if end - a >= settings.min_silence {
                result.push(Span { start: a, end });
            }
        }
    }
    if let Some(a) = start {
        if media.duration - a >= settings.min_silence {
            result.push(Span {
                start: a,
                end: media.duration,
            });
        }
    }
    progress(1., "Silence preview ready");
    Ok(result)
}
