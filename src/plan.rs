use crate::{engine::*, timeline::*};
use std::{collections::HashMap, sync::Arc};
#[derive(Clone, Debug)]
pub struct PreviewSpan {
    pub clip: usize,
    pub timeline_start: f64,
    pub media: Arc<Media>,
    pub start: f64,
    pub end: f64,
}
impl PreviewSpan {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }
}
#[derive(Clone, Debug, Default)]
pub struct PreviewPlan {
    pub spans: Vec<PreviewSpan>,
    pub duration: f64,
}
impl PreviewPlan {
    pub fn uncut(t: &Timeline) -> Self {
        let mut offset = 0.;
        let spans = t
            .clips
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let span = PreviewSpan {
                    clip: i,
                    timeline_start: offset,
                    media: c.media.clone(),
                    start: c.start,
                    end: c.end,
                };
                offset += c.duration();
                span
            })
            .collect();
        Self {
            spans,
            duration: offset,
        }
    }
    pub fn locate(&self, t: f64) -> Option<(usize, f64)> {
        for (i, s) in self.spans.iter().enumerate() {
            if t < s.timeline_start + s.len() - 0.00001 {
                return Some((i, (s.start + t - s.timeline_start).clamp(s.start, s.end)));
            }
        }
        None
    }
}
pub fn prepare(
    tools: &Tools,
    timeline: &Timeline,
    settings: &Settings,
    silence: bool,
    cancel: &Cancel,
    mut progress: impl FnMut(f64, &str),
) -> Result<PreviewPlan> {
    if !silence {
        return Ok(PreviewPlan::uncut(timeline));
    }
    let mut cache = HashMap::new();
    let mut result = PreviewPlan::default();
    let mut offset = 0.;
    for (i, c) in timeline.clips.iter().enumerate() {
        if !cache.contains_key(&c.media.path) && c.media.audio_count > 0 {
            let keep = match analyze(tools, &c.media, settings, cancel, |p, s| {
                progress((i as f64 + p) / timeline.clips.len() as f64, s)
            }) {
                Ok(a) => a.keep,
                Err(e) if e.starts_with("The entire video is below") => vec![],
                Err(e) => return Err(e),
            };
            cache.insert(c.media.path.clone(), keep);
        }
        let fallback = vec![Span {
            start: 0.,
            end: c.media.duration,
        }];
        let keep = cache.get(&c.media.path).unwrap_or(&fallback);
        let (mut start, mut end) = (c.start, c.end);
        if settings.quality == 0 && (start > 0.001 || end < c.media.duration - 0.001) {
            let keys = keyframes(tools, &c.media, cancel)?;
            let snap = |t: f64| {
                keys.iter()
                    .min_by(|a, b| (*a - t).abs().total_cmp(&(*b - t).abs()))
                    .copied()
                    .unwrap_or(t)
            };
            start = snap(start);
            end = snap(end);
        }
        for s in keep {
            let a = s.start.max(start);
            let b = s.end.min(end);
            if b - a > 0.001 {
                result.spans.push(PreviewSpan {
                    clip: i,
                    timeline_start: offset + (a - c.start),
                    media: c.media.clone(),
                    start: a,
                    end: b,
                });
                result.duration += b - a;
            }
        }
        offset += c.duration();
    }
    Ok(result)
}
