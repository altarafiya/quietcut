use quietcut::engine::*;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
fn main() {
    if let Err(e) = execute() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn execute() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    #[cfg(target_os = "linux")]
    match a.get(1).map(String::as_str) {
        Some("--version") => {
            println!("QuietCut {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--probe") => return probe_cli(&a[2..]),
        Some("--render-project" | "--plan-project") => return project_cli(&a),
        _ => {}
    }
    if a.get(1).is_some_and(|v| v == "--analyze") {
        let tools = Tools::find()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let mut s = Settings {
            quality: 2,
            ..Settings::default()
        };
        let preset = match a.get(3).map(String::as_str).unwrap_or("Balanced") {
            "Gentle" => 0,
            "Tight" => 2,
            _ => 1,
        };
        s.silence_preset(preset);
        if a.get(4).is_some_and(|x| x == "copy") {
            s.quality = 0;
        }
        let m = probe(
            &tools,
            std::path::Path::new(a.get(2).ok_or("Expected a video")?),
            &cancel,
        )?;
        let analysis = analyze(&tools, &m, &s, &cancel, |_, _| {})?;
        println!(
            "{}",
            serde_json::json!({"duration":analysis.duration,"keep":analysis.keep.iter().map(|s|vec![s.start,s.end]).collect::<Vec<_>>(),"removed":analysis.removed.iter().map(|s|vec![s.start,s.end]).collect::<Vec<_>>() })
        );
        return Ok(());
    }
    if a.get(1).is_some_and(|v| v == "--timeline") {
        return timeline_cli(&a);
    }
    if a.len() < 3 {
        return Err("Usage: quietcut-cli INPUT OUTPUT [copy|lossless|high|compact|hevc] [level] [threshold] [min_silence] [padding]".into());
    }
    let tools = Tools::find()?;
    let c = Arc::new(AtomicBool::new(false));
    let mut s = Settings::default();
    s.quality = match a.get(3).map(String::as_str).unwrap_or("copy") {
        "copy" => 0,
        "lossless" => 1,
        "high" => 2,
        "compact" => 3,
        "hevc" => 4,
        _ => return Err("Unknown quality mode".into()),
    };
    s.level = a.get(4).is_some_and(|v| v == "level");
    if let Some(v) = a.get(5) {
        s.adaptive = false;
        s.threshold = v.parse().map_err(|_| "Invalid threshold")?;
    }
    if let Some(v) = a.get(6) {
        s.min_silence = v.parse().map_err(|_| "Invalid minimum silence")?;
    }
    if let Some(v) = a.get(7) {
        s.padding = v.parse().map_err(|_| "Invalid padding")?;
    }
    let m = probe(&tools, &PathBuf::from(&a[1]), &c)?;
    let analysis = analyze(&tools, &m, &s, &c, |_, _| {})?;
    let out = export(
        &tools,
        &m,
        &analysis,
        &s,
        &PathBuf::from(&a[2]),
        &c,
        |_, _| {},
    )?;
    println!(
        "{}",
        serde_json::json!({"before_duration":m.duration,"after_duration":out.duration,"before_size":m.size,"after_size":out.size,"detected_silences":analysis.detected,"cuts":analysis.removed.iter().map(|s|vec![s.start,s.end]).collect::<Vec<_>>(),"output":out.path})
    );
    Ok(())
}
fn timeline_cli(a: &[String]) -> Result<()> {
    if a.len() < 4 {
        return Err("Usage: quietcut-cli --timeline PROJECT.json OUTPUT [copy|lossless|high|compact|hevc] [level] [silence]".into());
    }
    let tools = Tools::find()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut s = Settings::default();
    s.quality = match a.get(4).map(String::as_str).unwrap_or("high") {
        "copy" => 0,
        "lossless" => 1,
        "high" => 2,
        "compact" => 3,
        "hevc" => 4,
        _ => return Err("Unknown mode".into()),
    };
    s.level = a.get(5).is_some_and(|v| v == "level");
    let silence = a.get(6).is_some_and(|v| v == "silence");
    s.silence_preset(match a.get(7).map(String::as_str) {
        Some("Gentle") => 0,
        Some("Tight") => 2,
        _ => 1,
    });
    let v = read_project(&a[2])?;
    let t = load_timeline(&v, &tools, &cancel)?;
    let o = export_timeline(
        &tools,
        &t,
        &s,
        silence,
        std::path::Path::new(&a[3]),
        &cancel,
        |_, _| {},
    )?;
    println!(
        "{}",
        serde_json::json!({"before_duration":t.duration(),"after_duration":o.duration,"after_size":o.size,"output":o.path})
    );
    Ok(())
}

fn read_project(path: &str) -> Result<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn load_timeline(
    v: &serde_json::Value,
    tools: &Tools,
    cancel: &Cancel,
) -> Result<quietcut::timeline::Timeline> {
    use quietcut::timeline::{Clip, Timeline};
    let mut t = Timeline::default();
    // A split timeline can reference one source thousands of times. Probe it
    // once per invocation, as the native GUI already does on import.
    let mut media = std::collections::HashMap::<PathBuf, Arc<Media>>::new();
    for c in v["clips"].as_array().ok_or("Expected clips array")? {
        let path = PathBuf::from(c["path"].as_str().ok_or("Expected clip path")?);
        let m = if let Some(m) = media.get(&path) {
            m.clone()
        } else {
            let m = Arc::new(probe(tools, &path, cancel)?);
            media.insert(path, m.clone());
            m
        };
        let start = c["start"].as_f64().unwrap_or(0.);
        let end = c["end"].as_f64().unwrap_or(m.duration);
        if start < 0. || end > m.duration || end <= start {
            return Err("Invalid clip bounds".into());
        }
        t.clips.push(Clip {
            media: m,
            start,
            end,
        });
    }
    Ok(t)
}

#[cfg(target_os = "linux")]
fn probe_cli(paths: &[String]) -> Result<()> {
    use serde_json::json;
    let tools = Tools::find()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let results: Vec<_> = paths
        .iter()
        .map(
            |path| match probe(&tools, std::path::Path::new(path), &cancel) {
                Ok(media) => json!({"path": path, "duration": media.duration, "fps": media.fps, "size": media.size, "audio_tracks": media.audio_count}),
                Err(error) => json!({"path": path, "error": error}),
            },
        )
        .collect();
    println!("{}", json!({"media": results}));
    Ok(())
}

#[cfg(target_os = "linux")]
fn project_cli(args: &[String]) -> Result<()> {
    use serde_json::json;
    use std::{
        io::{self, Read, Write},
        path::Path,
        sync::atomic::Ordering,
    };
    let render = args[1] == "--render-project";
    if args.len() != if render { 4 } else { 3 } {
        return Err("Usage: quietcut-cli --plan-project PROJECT.json or --render-project PROJECT.json OUTPUT".into());
    }
    let project = read_project(&args[2])?;
    let mut settings: Settings =
        serde_json::from_value(project["settings"].clone()).map_err(|e| e.to_string())?;
    let silence = project["remove_silence"].as_bool().unwrap_or(false);
    let audio_only = project["audio_only"].as_bool().unwrap_or(false);
    if audio_only {
        settings.quality = 2; // Audio cuts are independent of video keyframes.
    }
    settings.exact_silence(silence);
    let tools = Tools::find()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let request = cancel.clone();
    std::thread::spawn(move || {
        if io::stdin().read_exact(&mut [0]).is_ok() {
            request.store(true, Ordering::Relaxed);
        }
    });
    let timeline = load_timeline(&project, &tools, &cancel)?;
    let progress = |fraction, status: &str| {
        let mut stdout = io::stdout().lock();
        if writeln!(
            stdout,
            "{}",
            json!({"progress": fraction, "status": status})
        )
        .and_then(|_| stdout.flush())
        .is_err()
        {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    if render {
        let output = Path::new(&args[3]);
        let out = if audio_only {
            export_audio(
                &tools, &timeline, &settings, silence, output, &cancel, progress,
            )?
        } else {
            export_timeline(
                &tools, &timeline, &settings, silence, output, &cancel, progress,
            )?
        };
        println!(
            "{}",
            json!({"output": out.path, "duration": out.duration, "size": out.size})
        );
    } else {
        let plan =
            quietcut::plan::prepare(&tools, &timeline, &settings, silence, &cancel, progress)?;
        let spans: Vec<_> = plan
            .spans
            .iter()
            .map(|span| {
                json!({
                    "clip": span.clip, "timeline_start": span.timeline_start,
                    "start": span.start, "end": span.end
                })
            })
            .collect();
        println!("{}", json!({"spans": spans, "duration": plan.duration}));
    }
    Ok(())
}
