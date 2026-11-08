"""Frame hashes, exact linked samples, VFR timing, NTSC and coarse clocks."""
import array
import bisect
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / ("target/release/quietcut-cli.exe" if os.name == "nt" else "target/release/quietcut-cli")
ENV = dict(os.environ, QUIETCUT_PROFILE='1')
RATE = 48000


def run(*args, env=None):
    result = subprocess.run([str(a) for a in args], capture_output=True,
                            env=env, creationflags=0x08000000 if os.name == "nt" else 0)
    assert result.returncode == 0, result.stderr.decode(errors='replace')
    return result


def clock(path):
    result = json.loads(run('ffprobe', '-v', 'error', '-select_streams', 'V:0',
                            '-show_streams', '-show_packets', '-show_format',
                            '-of', 'json', path).stdout)
    s = result['streams'][0]
    num, den = map(int, s['time_base'].split('/'))
    packets = sorted(result['packets'], key=lambda x: int(x['pts']))
    origin = float(result['format'].get('start_time', 0))
    pts = [int(p['pts']) * num / den - origin for p in packets]
    end = (int(packets[-1]['pts']) + int(packets[-1]['duration'])) * num / den - origin
    return pts, end


def hashes(path):
    text = run('ffmpeg', '-v', 'error', '-i', path, '-map', '0:v:0',
               '-enc_time_base:v', 'demux', '-fps_mode', 'passthrough',
               '-f', 'framemd5', '-').stdout.decode()
    return [line.split(',')[-1].strip() for line in text.splitlines() if not line.startswith('#')]


def samples(path, track=0):
    return array.array('d', run('ffmpeg', '-v', 'error', '-i', path,
                               '-map', f'0:a:{track}',
                               '-ar', RATE, '-ac', '1', '-f', 'f64le', '-').stdout)


def round_sample(t):
    return math.floor(t * RATE + .5)


def expected(source, clips):
    pts, end = clock(source)
    frames = hashes(source)
    audio = samples(source)
    timing = json.loads(run('ffprobe', '-v', 'error', '-show_streams', '-show_format',
                             '-of', 'json', source).stdout)
    audio_stream = next(s for s in timing['streams'] if s['codec_type'] == 'audio')
    lead = float(audio_stream.get('start_time', 0)) - float(timing['format'].get('start_time', 0))
    if lead > 0:
        audio = array.array('d', [0.] * round_sample(lead)) + audio
    selected, output_pts, selected_audio = [], [], array.array('d')
    for clip in clips:
        a = bisect.bisect_left(pts, clip['start'] - 1e-9)
        b = bisect.bisect_left(pts, clip['end'] - 1e-9)
        if b <= a:
            continue
        start_sample, end_sample = round_sample(pts[a]), round_sample(pts[b] if b < len(pts) else end)
        offset = len(selected_audio) / RATE
        selected.extend(frames[a:b])
        output_pts.extend(offset + t - pts[a] for t in pts[a:b])
        chunk = audio[start_sample:end_sample]
        chunk.extend([0.] * (end_sample-start_sample-len(chunk)))
        selected_audio.extend(chunk)
    return selected, output_pts, selected_audio


def db(values):
    return 20 * math.log10(max(1e-12, math.sqrt(sum(x*x for x in values) / len(values))))


with tempfile.TemporaryDirectory(prefix='quietcut-timestamps-') as folder:
    p = Path(folder)
    report = []
    raw = p / 'impulses.f64'
    # Distinct nonperiodic samples expose even one-sample cut errors.
    wave = array.array('d', (.25 * math.sin(n * .017) + .15 * math.sin(n * .061)
                             for n in range(RATE * 13)))
    raw.write_bytes(wave.tobytes())
    base = p / "original 视频's.mp4"
    run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=60:duration=12',
        '-f', 'f64le', '-ar', RATE, '-ac', '1', '-i', raw, '-t', 12,
        '-c:v', 'libx264', '-preset', 'veryfast', '-g', '600', '-c:a', 'alac', base)
    sources = []
    variable = p / 'variable.mp4'
    run('ffmpeg', '-v', 'error', '-i', base, '-vf', "select='not(mod(n,7))+gt(mod(n,11),1)'",
        '-fps_mode', 'vfr', '-c:v', 'libx264', '-preset', 'veryfast', '-g', 600,
        '-c:a', 'copy', variable)
    sources.append(('VFR', variable))
    ntsc = p / 'ntsc.mp4'
    run('ffmpeg', '-v', 'error', '-i', base, '-vf', 'fps=30000/1001',
        '-c:v', 'libx264', '-preset', 'veryfast', '-g', 600, '-c:a', 'copy', ntsc)
    sources.append(('NTSC', ntsc))
    coarse = p / 'coarse.mkv'
    run('ffmpeg', '-v', 'error', '-i', variable, '-c', 'copy', coarse)
    sources.append(('coarse-clock-no-frame-count', coarse))
    offset = p / 'offset.mp4'
    run('ffmpeg', '-v', 'error', '-itsoffset', .125, '-i', variable, '-c', 'copy', offset)
    sources.append(('nonzero-start-time', offset))
    delayed = p / 'delayed-audio.mp4'
    run('ffmpeg', '-v', 'error', '-i', variable, '-i', base,
        '-filter_complex', '[1:a]asetpts=PTS+0.137/TB[a]', '-map', '0:v', '-map', '[a]',
        '-c:v', 'copy', '-c:a', 'alac', delayed)
    sources.append(('delayed-audio', delayed))
    project = p / 'project.json'
    for name, source in sources:
        clips = [{'path': str(source), 'start': .017+n*.08, 'end': .074+n*.08}
                 for n in range(140)]
        project.write_text(json.dumps({'clips': clips}), encoding='utf-8')
        expected_frames, expected_pts, expected_samples = expected(source, clips)
        for gpu in (True, False):
            env = dict(ENV)
            if not gpu:
                env['QUIETCUT_DISABLE_GPU'] = '1'
            output = p / f'{name}-{gpu}.mkv'
            result = run(CLI, '--timeline', project, output, 'lossless', env=env)
            assert b'Single pass:' in result.stderr and b'timestamp-aware=true' in result.stderr, result.stderr.decode()
            assert b'fallback' not in result.stderr, result.stderr.decode()
            assert hashes(output) == expected_frames, f'{name}: retained pictures changed or moved'
            actual_audio = samples(output)
            assert len(actual_audio) == len(expected_samples), (name, len(actual_audio), len(expected_samples))
            error = max(abs(a-b) for a, b in zip(actual_audio, expected_samples))
            assert error < 1e-12, (name, error)
            actual_pts, _ = clock(output)
            assert len(actual_pts) == len(expected_pts)
            drift = max(abs(a-b) for a,b in zip(actual_pts, expected_pts))
            assert drift <= .000501, (name, drift)  # Matroska's millisecond clock
            report.append({'test': name, 'gpu': gpu, 'retained_frames': len(expected_frames),
                           'exact_audio_samples': len(actual_audio), 'max_timestamp_error': drift})

    # Multiple linked audio tracks use independent, sample-exact segment routing.
    two_tracks = p / 'two-tracks.mp4'
    run('ffmpeg', '-v', 'error', '-i', ntsc, '-map', '0:v', '-map', '0:a', '-map', '0:a',
        '-c', 'copy', two_tracks)
    clips = [{'path': str(two_tracks), 'start': .017+n*.08, 'end': .074+n*.08} for n in range(140)]
    project.write_text(json.dumps({'clips': clips}))
    expected_frames, _, expected_samples = expected(two_tracks, clips)
    output = p / 'two-tracks.mkv'
    result = run(CLI, '--timeline', project, output, 'lossless', env=ENV)
    assert b'timestamp-aware=true' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    assert hashes(output) == expected_frames
    for track in (0,1):
        actual = samples(output, track)
        assert len(actual) == len(expected_samples)
        assert max(abs(a-b) for a,b in zip(actual, expected_samples)) < 1e-12
    report.append({'test': 'two-linked-tracks', 'exact_samples_per_track': len(expected_samples)})

    # Entire VFR input, video only: preserve every picture and original intervals.
    noaudio = p / 'no-audio.mp4'
    run('ffmpeg', '-v', 'error', '-i', variable, '-an', '-c:v', 'copy', noaudio)
    project.write_text(json.dumps({'clips': [{'path': str(noaudio)}]}))
    output = p / 'no-audio.mkv'
    result = run(CLI, '--timeline', project, output, 'lossless', env=ENV)
    assert b'timestamp-aware=true' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    assert hashes(output) == hashes(noaudio)
    actual_pts, _ = clock(output)
    source_pts, _ = clock(noaudio)
    assert max(abs(a-b) for a,b in zip(actual_pts,source_pts)) < .000501
    report.append({'test': 'video-only-all-original-frames', 'frames': len(actual_pts)})

    # Check the actual AAC result with NTSC timing, silence removal and leveling.
    phrases = p / 'phrases.f64'
    wave = array.array('d', ((.08 if 1 <= n/RATE < 3 or 7 <= n/RATE < 9 else
                             .7 if 4 <= n/RATE < 6 else 0.) * math.sin(2*math.pi*440*n/RATE)
                            for n in range(10*RATE)))
    phrases.write_bytes(wave.tobytes())
    source = p / 'ntsc-phrases.mp4'
    run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=size=320x180:rate=30000/1001:duration=10',
        '-f', 'f64le', '-ar', RATE, '-ac', 1, '-i', phrases,
        '-c:v', 'libx264', '-preset', 'veryfast', '-c:a', 'alac', source)
    project.write_text(json.dumps({'clips': [{'path': str(source)}]}))
    output = p / 'leveled.mp4'
    result = run(CLI, '--timeline', project, output, 'high', 'level', 'silence', env=ENV)
    assert b'timestamp-aware=true' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    plan = json.loads(run(CLI, '--analyze', source, 'Balanced').stdout)
    pts, end = clock(source)
    audio = samples(output)
    offset = 0
    levels, changes = [], []
    for a,b in plan['keep']:
        a = pts[bisect.bisect_left(pts, a-1e-9)]
        i = bisect.bisect_left(pts, b-1e-9)
        b = pts[i] if i < len(pts) else end
        phrase = next((x,y) for x,y in [(1,3),(4,6),(7,9)] if a <= x < b)
        start = offset/RATE + phrase[0] - a
        level = db(audio[int((start+.5)*RATE):int((start+1)*RATE)])
        onset = db(audio[int((start+.03)*RATE):int((start+.1)*RATE)])
        tail = db(audio[int((start+1.7)*RATE):int((start+1.95)*RATE)])
        assert abs(onset-level) < .5, (onset, level)
        levels.append(level)
        changes.append(abs(tail-level))
        offset += round_sample(b)-round_sample(a)
    assert max(levels)-min(levels) < 1.5, levels
    assert max(changes) < .25, changes
    report.append({'test': 'NTSC-silence-and-level', 'level_spread_db': max(levels)-min(levels),
                   'max_pre_cut_change_db': max(changes)})
    assert not list(p.glob('.quietcut-*')), 'Export workspace was left behind'
    print(json.dumps({'passed': True, 'results': report}, indent=2))
