use crate::engine::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Clip {
    pub media: Arc<Media>,
    pub start: f64,
    pub end: f64,
}
impl Clip {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub clips: Vec<Clip>,
}
impl Timeline {
    pub fn duration(&self) -> f64 {
        self.clips.iter().map(Clip::duration).sum()
    }
    pub fn bytes(&self) -> u64 {
        self.clips
            .iter()
            .map(|c| (c.media.size as f64 * c.duration() / c.media.duration) as u64)
            .sum()
    }
    pub fn add(&mut self, media: Media) {
        let end = media.duration;
        self.clips.push(Clip {
            media: Arc::new(media),
            start: 0.,
            end,
        });
    }
    pub fn offset(&self, index: usize) -> f64 {
        self.clips.iter().take(index).map(Clip::duration).sum()
    }
    pub fn locate(&self, t: f64) -> Option<(usize, f64)> {
        let mut offset = 0.;
        for (i, c) in self.clips.iter().enumerate() {
            if t < offset + c.duration() - 0.000001 || i + 1 == self.clips.len() {
                return Some((i, (c.start + t - offset).clamp(c.start, c.end)));
            }
            offset += c.duration();
        }
        None
    }
    pub fn split(&mut self, t: f64) -> bool {
        let Some((i, source)) = self.locate(t) else {
            return false;
        };
        let clip = &self.clips[i];
        let frame = 1. / clip.media.fps;
        let source = (source * clip.media.fps).round() / clip.media.fps;
        if source - clip.start < frame * 0.9 || clip.end - source < frame * 0.9 {
            return false;
        }
        let mut tail = clip.clone();
        tail.start = source;
        self.clips[i].end = source;
        self.clips.insert(i + 1, tail);
        true
    }
    pub fn remove(&mut self, i: usize) {
        if i < self.clips.len() {
            self.clips.remove(i);
        }
    }
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from >= self.clips.len() || to >= self.clips.len() || from == to {
            return;
        }
        let c = self.clips.remove(from);
        self.clips.insert(to, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn media() -> Media {
        Media {
            path: "test.mp4".into(),
            duration: 10.,
            size: 1000,
            width: 320,
            height: 180,
            video_codec: "h264".into(),
            audio_count: 1,
            pixel_format: "yuv420p".into(),
            fps: 30.,
            start_time: 0.,
            color: vec![],
            audio_specs: vec![(48000, 2)],
        }
    }
    #[test]
    fn splits_map_back_to_source_without_lost_frames() {
        let mut t = Timeline::default();
        t.add(media());
        assert!(t.split(4.));
        assert_eq!(t.clips.len(), 2);
        assert_eq!(t.duration(), 10.);
        assert_eq!(t.locate(5.), Some((1, 5.)));
        t.remove(0);
        assert_eq!(t.locate(0.), Some((0, 4.)));
        assert_eq!(t.duration(), 6.);
    }
    #[test]
    fn rejects_empty_splits() {
        let mut t = Timeline::default();
        t.add(media());
        assert!(!t.split(0.));
        assert!(!t.split(10.));
        assert_eq!(t.duration(), 10.);
    }
    #[test]
    fn reorder_preserves_bounds() {
        let mut t = Timeline::default();
        t.add(media());
        t.split(4.);
        t.reorder(1, 0);
        assert_eq!(t.clips[0].start, 4.);
        assert_eq!(t.duration(), 10.);
    }
    #[test]
    fn preview_seek_skips_gaps_and_preserves_source_positions() {
        use crate::plan::{PreviewPlan, PreviewSpan};
        let media = Arc::new(media());
        let plan = PreviewPlan {
            spans: vec![
                PreviewSpan {
                    clip: 0,
                    timeline_start: 1.,
                    media: media.clone(),
                    start: 1.,
                    end: 3.,
                },
                PreviewSpan {
                    clip: 0,
                    timeline_start: 6.,
                    media,
                    start: 6.,
                    end: 8.,
                },
            ],
            duration: 4.,
        };
        assert_eq!(plan.locate(0.), Some((0, 1.)));
        assert_eq!(plan.locate(2.), Some((0, 2.)));
        assert_eq!(plan.locate(3.), Some((1, 6.)));
        assert_eq!(plan.locate(5.), Some((1, 6.)));
        assert_eq!(plan.locate(7.), Some((1, 7.)));
        assert_eq!(plan.locate(8.), None);
    }
}
