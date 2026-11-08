//! Small export workflow helpers; the media engine remains unchanged.
use crate::engine::Result;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

pub fn file_stem(input: &str) -> Result<String> {
    let mut stem = input.trim();
    if stem.len() >= 4
        && [".mp4", ".mkv"]
            .iter()
            .any(|ext| stem.to_ascii_lowercase().ends_with(ext))
    {
        stem = &stem[..stem.len() - 4];
    }
    if stem.is_empty() {
        return Err("Enter a file name.".into());
    }
    if stem.encode_utf16().count() > 240 {
        return Err("Use a shorter file name (240 characters or fewer).".into());
    }
    if stem.ends_with(['.', ' '])
        || matches!(stem, "." | "..")
        || stem
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err(
            "Use a file name without trailing dots or these characters: < > : \" / \\ | ? *".into(),
        );
    }
    let device = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || ["COM", "LPT"].iter().any(|prefix| {
            device
                .strip_prefix(prefix)
                .is_some_and(|s| s.len() == 1 && matches!(s.as_bytes()[0], b'1'..=b'9'))
        });
    if reserved {
        return Err("That name is reserved by Windows. Choose another name.".into());
    }
    Ok(stem.into())
}

pub fn proposed_output(folder: &Path, name: &str, extension: &str) -> Result<PathBuf> {
    let stem = file_stem(name)?;
    for n in 0_u64.. {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{n}")
        };
        let path = folder.join(format!("{stem}{suffix}.{extension}"));
        if !path
            .try_exists()
            .map_err(|e| format!("Cannot check the output folder: {e}"))?
        {
            return Ok(path);
        }
    }
    unreachable!()
}

#[derive(Default)]
pub struct ProgressEstimate {
    samples: VecDeque<(f64, f64)>,
}
impl ProgressEstimate {
    pub fn observe(&mut self, seconds: f64, fraction: f64) {
        if !seconds.is_finite() || !fraction.is_finite() {
            return;
        }
        if fraction < 0.18
            || self
                .samples
                .back()
                .is_some_and(|(t, p)| seconds < *t || fraction < *p - 0.001)
        {
            self.samples.clear();
        }
        // Preparation jumps to 18%; it does not measure encoding throughput.
        if fraction < 0.18 {
            return;
        }
        self.samples.push_back((seconds, fraction.clamp(0., 1.)));
        while self.samples.len() > 128
            || (self.samples.len() > 2 && seconds - self.samples[0].0 > 20.)
        {
            self.samples.pop_front();
        }
    }
    pub fn remaining(&self, seconds: f64) -> Option<f64> {
        let (start, first) = *self.samples.front()?;
        let (_, current) = *self.samples.back()?;
        let elapsed = seconds - start;
        let advance = current - first;
        if self.samples.len() < 3
            || elapsed < 1.
            || advance < 0.001
            || current >= 1.
            || !seconds.is_finite()
        {
            return None;
        }
        let remaining = (1. - current) * elapsed / advance;
        (remaining.is_finite() && remaining >= 0.).then_some(remaining)
    }
}

pub fn short_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.).ceil() as u64;
    if seconds >= 3600 {
        format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60)
    } else if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_names_and_unicode() {
        assert_eq!(file_stem("  أسبوع 3.MP4  ").unwrap(), "أسبوع 3");
        assert_eq!(file_stem("Lecture.v2").unwrap(), "Lecture.v2");
        for bad in [
            "",
            ".",
            "..",
            "CON",
            "nul.txt",
            "COM1",
            "LPT9.mkv",
            "../video",
            "a/b",
            "C:\\video",
            "bad?",
            "end.",
            "bad\nname",
        ] {
            assert!(file_stem(bad).is_err(), "Accepted {bad:?}");
        }
    }
    #[test]
    fn collisions_do_not_overwrite_inputs_or_exports() {
        let p = std::env::temp_dir().join(format!("quietcut-names-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("lecture.mp4"), b"source").unwrap();
        std::fs::write(p.join("lecture-1.mp4"), b"export").unwrap();
        assert_eq!(
            proposed_output(&p, "lecture.mp4", "mp4").unwrap(),
            p.join("lecture-2.mp4")
        );
        assert_eq!(
            proposed_output(&p, "lecture.mp4", "mkv").unwrap(),
            p.join("lecture.mkv")
        );
        assert_eq!(std::fs::read(p.join("lecture.mp4")).unwrap(), b"source");
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn estimate_ignores_preparation_and_uses_measured_progress() {
        let mut p = ProgressEstimate::default();
        for (t, f) in [(0., 0.), (1., 0.1), (3., 0.18)] {
            p.observe(t, f);
        }
        assert_eq!(p.remaining(3.), None);
        p.observe(4., 0.2);
        p.observe(5., 0.22);
        assert!((p.remaining(5.).unwrap() - 39.).abs() < 0.001);
        assert!(p.remaining(10.).unwrap() > p.remaining(5.).unwrap());
        p.observe(11., 0.); // driver fallback must discard the old speed estimate
        assert_eq!(p.remaining(11.), None);
        p.observe(12., 0.18);
        p.observe(13., 0.5);
        p.observe(14., 1.);
        assert_eq!(p.remaining(14.), None);
    }
}
