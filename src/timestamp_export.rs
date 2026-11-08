//! Single-session export for variable frame timing and fractional audio grids.
//! Packet PTS determine the actual retained frames; never assume a nominal FPS.
use super::*;

struct Clock {
    num: u64,
    den: u64,
    pts: Vec<i64>,
    end: i64,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn read_clock(tools: &Tools, m: &Media, cancel: &Cancel) -> Result<Option<Clock>> {
    // These codecs use one packet per picture. Reject damaged, interlaced,
    // duplicate-PTS or unreasonably large inputs, leaving the compatible path.
    if !["h264", "hevc", "vp9", "mpeg2video", "vc1"].contains(&m.video_codec.as_str()) {
        return Ok(None);
    }
    let mut argv = args(&[
        "-v",
        "error",
        "-select_streams",
        "V:0",
        "-show_entries",
        "stream=time_base,nb_frames,field_order",
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
    let Some((num, den)) = rational(&s["time_base"]) else {
        return Ok(None);
    };
    if !["progressive", "unknown"].contains(&s["field_order"].as_str().unwrap_or("unknown")) {
        return Ok(None);
    }
    let expected = number(s.get("nb_frames"));
    let mut packets = Vec::new();
    let mut valid = true;
    let mut argv = args(&[
        "-v",
        "error",
        "-select_streams",
        "V:0",
        "-show_entries",
        "packet=pts,duration",
        "-of",
        "csv=p=0",
    ]);
    argv.push(m.path.as_os_str().into());
    run(&tools.ffprobe, &argv, cancel, |out, line| {
        if !out || line.trim().is_empty() {
            return;
        }
        let mut fields = line.split(',');
        match (
            fields.next().and_then(|s| s.parse::<i64>().ok()),
            fields.next().and_then(|s| s.parse::<i64>().ok()),
        ) {
            (Some(pts), duration) if packets.len() < 2_000_000 => packets.push((pts, duration)),
            _ => valid = false,
        }
    })?;
    if !valid || packets.is_empty() || expected.is_some_and(|n| n as usize != packets.len()) {
        return Ok(None);
    }
    packets.sort_unstable_by_key(|p| p.0);
    if packets.windows(2).any(|p| p[0].0 >= p[1].0) {
        return Ok(None);
    }
    let (last, duration) = *packets.last().unwrap();
    let Some(end) = duration
        .filter(|d| *d > 0)
        .and_then(|d| last.checked_add(d))
    else {
        return Ok(None);
    };
    let first = packets[0].0 as f64 * num as f64 / den as f64 - m.start_time;
    let last = end as f64 * num as f64 / den as f64 - m.start_time;
    if !(-0.001..=1.).contains(&first) || last > m.duration + 0.08 || last <= 0. {
        return Ok(None);
    }
    Ok(Some(Clock {
        num,
        den,
        pts: packets.into_iter().map(|p| p.0).collect(),
        end,
    }))
}

/// A balanced lookup by retained-frame ordinal avoids deep expressions and
/// preserves every frame's source timing within each kept interval.
fn offset_expression(offsets: &[(u64, i64)]) -> String {
    if offsets.len() == 1 {
        return offsets[0].1.to_string();
    }
    let mid = offsets.len() / 2;
    format!(
        "if(lt(N,{}),{},{})",
        offsets[mid].0,
        offset_expression(&offsets[..mid]),
        offset_expression(&offsets[mid..])
    )
}

pub(super) fn build(
    tools: &Tools,
    m: &Media,
    plans: &[(Arc<Media>, Span)],
    level: bool,
    format: &str,
    cancel: &Cancel,
) -> Result<Option<RenderPlan>> {
    let Some(clock) = read_clock(tools, m, cancel)? else {
        return Ok(None);
    };
    let Some(tb) = (clock.den / gcd(clock.den, 48_000))
        .checked_mul(48_000)
        .filter(|tb| *tb <= 1_000_000_000)
    else {
        return Ok(None);
    };
    let tick = clock.num * (tb / clock.den);
    let sample_tick = tb / 48_000;
    // Keep all integer arithmetic exactly representable by FFmpeg expressions.
    if clock
        .end
        .unsigned_abs()
        .checked_mul(tick)
        .is_none_or(|n| n > (1 << 52))
    {
        return Ok(None);
    }
    let sample = |pts: i64| {
        ((pts as f64 * clock.num as f64 / clock.den as f64 - m.start_time) * 48_000.)
            .round()
            .max(0.) as u64
    };
    let mut ranges: Vec<(u64, u64)> = vec![];
    for (_, span) in plans {
        let index = |seconds: f64| {
            clock.pts.partition_point(|p| {
                *p as f64 * clock.num as f64 / clock.den as f64 - m.start_time
                    < seconds - 0.000000001
            }) as u64
        };
        let (a, b) = (index(span.start), index(span.end));
        if b <= a {
            continue;
        }
        if let Some(last) = ranges.last_mut() {
            if last.1 >= a {
                last.1 = last.1.max(b);
                continue;
            }
        }
        ranges.push((a, b));
    }
    if ranges.is_empty() {
        return Ok(None);
    }
    let end_pts = |b: u64| clock.pts.get(b as usize).copied().unwrap_or(clock.end);
    let mut audio = Vec::new();
    let mut offsets = Vec::new();
    let mut count = 0;
    let mut samples = 0;
    for &(a, b) in &ranges {
        let first = clock.pts[a as usize];
        let (sa, sb) = (sample(first), sample(end_pts(b)));
        if sb <= sa {
            return Ok(None);
        }
        audio.push((sa, sb));
        offsets.push((count, first * tick as i64 - (samples * sample_tick) as i64));
        samples += sb - sa;
        count += b - a;
    }
    let mut graph = format!(
        "[0:V:0]select='{}',settb=1/{tb},setpts='PTS-({})',{format},setsar=1[v]",
        selection(&ranges),
        offset_expression(&offsets),
    );
    // Keep a single audio sink. Independent anullsinks eagerly drain future
    // segments, which can hold the output at startup on long edited recordings.
    // Empty segments stay in the ordered concat; reset their EOF timestamp to
    // zero so a discarded interval adds neither samples nor an output gap.
    let mut boundaries: Vec<_> = audio
        .iter()
        .flat_map(|(a, b)| [*a, *b])
        .filter(|n| *n > 0)
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    for track in 0..m.audio_count {
        let level_filter = if level {
            format!(",{LEVEL_FILTER}")
        } else {
            String::new()
        };
        graph.push_str(&format!(
            ";[0:a:{track}]asetpts=PTS-{:.9}/TB,aresample=48000:async=1:first_pts=0,apad,atrim=end_sample={},asetpts=PTS-STARTPTS{level_filter},asegment=samples='{}'",
            m.start_time, audio.last().unwrap().1,
            boundaries.iter().map(u64::to_string).collect::<Vec<_>>().join("|"),
        ));
        for i in 0..=boundaries.len() {
            graph.push_str(&format!("[s{track}_{i}]"));
        }
        for (i, start) in std::iter::once(0)
            .chain(boundaries.iter().copied())
            .enumerate()
        {
            let discard = if audio.binary_search_by_key(&start, |(a, _)| *a).is_ok() {
                ""
            } else {
                "aselect=0,"
            };
            graph.push_str(&format!(
                ";[s{track}_{i}]{discard}asetpts=N/SR/TB[k{track}_{i}]"
            ));
        }
        graph.push(';');
        let segments = boundaries.len() + 1;
        for i in 0..segments {
            graph.push_str(&format!("[k{track}_{i}]"));
        }
        graph.push_str(&format!(
            "concat=n={segments}:v=0:a=1,asetpts=N/SR/TB[a{track}]"
        ));
    }
    Ok(Some(RenderPlan {
        graph,
        duration: samples as f64 / 48_000.,
        input_duration: end_pts(ranges.last().unwrap().1) as f64 * clock.num as f64
            / clock.den as f64
            - m.start_time,
        time_base: tb,
        frames: count,
        ranges: ranges.len(),
        copy_timestamps: true,
    }))
}
