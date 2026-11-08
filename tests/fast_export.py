"""Single-pass accuracy, source-order, audio alignment and fallback regressions."""
import array
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / ("target/release/quietcut-cli.exe" if os.name == "nt" else "target/release/quietcut-cli")


def run(*args, env=None, check=True):
    return subprocess.run([str(a) for a in args], capture_output=True, check=check,
                          env=env, creationflags=0x08000000 if os.name == "nt" else 0)


def hashes(path):
    text = run('ffmpeg', '-v', 'error', '-i', path, '-map', '0:v:0',
               '-enc_time_base:v', 'demux', '-fps_mode', 'passthrough',
               '-f', 'framemd5', '-').stdout.decode()
    return [line.split(',')[-1].strip() for line in text.splitlines() if not line.startswith('#')]


def samples(path, track=0):
    return array.array('d', run('ffmpeg', '-v', 'error', '-i', path,
                               '-map', f'0:a:{track}', '-ac', '1', '-ar', '48000',
                               '-f', 'f64le', '-').stdout)


def db(values):
    return 20 * math.log10(max(1e-12, math.sqrt(sum(x*x for x in values) / len(values))))


with tempfile.TemporaryDirectory(prefix='quietcut-fast-') as folder:
    p = Path(folder)
    source = p / "CFR 视频 it's.mp4"
    # Each video frame has a different image; frame-boundary audio impulses prove
    # the chosen samples stay attached to the chosen image across many cuts.
    rate, fps, seconds = 48000, 60, 12
    raw = p / 'grid.f64'
    wave = array.array('d', (0.4 if n % 800 == 80 else 0. for n in range(rate * seconds)))
    raw.write_bytes(wave.tobytes())
    run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', f'testsrc2=size=320x180:rate={fps}:duration={seconds}',
        '-f', 'f64le', '-ar', rate, '-ac', '1', '-i', raw,
        '-c:v', 'libx264', '-preset', 'veryfast', '-g', '600', '-c:a', 'alac', source)
    source_frames = hashes(source)
    source_audio = samples(source)
    # Boundaries deliberately miss the source frame grid and every keyframe.
    clips = [{'path': str(source), 'start': .017 + n * .08, 'end': .074 + n * .08}
             for n in range(140)]
    project = p / 'edits.json'
    project.write_text(json.dumps({'clips': clips}), encoding='utf-8')
    ranges = [(math.ceil(c['start'] * fps - 1e-6), math.ceil(c['end'] * fps - 1e-6)) for c in clips]
    expected_frames = [h for a, b in ranges for h in source_frames[a:b]]
    expected_audio = array.array('d', (x for a, b in ranges for x in source_audio[a*800:b*800]))
    report = []
    for gpu in (True, False):
        env = os.environ.copy()
        env['QUIETCUT_PROFILE'] = '1'
        if not gpu:
            env['QUIETCUT_DISABLE_GPU'] = '1'
        output = p / f'lossless-{gpu}.mkv'
        result = run(CLI, '--timeline', project, output, 'lossless', env=env)
        assert b'Single pass:' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
        assert hashes(output) == expected_frames, 'Retained video frames changed or moved'
        actual_audio = samples(output)
        assert len(actual_audio) == len(expected_audio), (len(actual_audio), len(expected_audio))
        assert max(abs(a-b) for a, b in zip(actual_audio, expected_audio)) < 1e-12, 'Sample shift at a cut'
        report.append({'test': f'lossless-{"GPU" if gpu else "CPU"}', 'frames': len(expected_frames),
                       'aligned_audio_samples': len(actual_audio)})

    # Exercise actual silence cuts and leveling together through the new graph.
    # Quiet and loud phrases have silence between them; compressor recovery must
    # not fade down a quiet phrase ahead of its cut or duck it after a cut.
    tone = p / 'tone.f64'
    wave = array.array('d', ((.08 if 1 <= n/rate < 3 or 7 <= n/rate < 9 else
                             .7 if 4 <= n/rate < 6 else 0.) * math.sin(2*math.pi*440*n/rate)
                            for n in range(10 * rate)))
    tone.write_bytes(wave.tobytes())
    leveled_source = p / 'phrases.mp4'
    run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=size=320x180:rate=60:duration=10',
        '-f', 'f64le', '-ar', rate, '-ac', '1', '-i', tone,
        '-c:v', 'libx264', '-preset', 'veryfast', '-c:a', 'alac', leveled_source)
    project.write_text(json.dumps({'clips': [{'path': str(leveled_source)}]}))
    env = dict(os.environ, QUIETCUT_PROFILE='1')
    output = p / 'leveled.mp4'
    result = run(CLI, '--timeline', project, output, 'high', 'level', 'silence', env=env)
    assert b'Single pass:' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    plan = json.loads(run(CLI, '--analyze', leveled_source, 'Balanced').stdout)
    output_audio = samples(output)
    offset = 0
    levels, gains = [], []
    for a, b in plan['keep']:
        a, b = math.ceil(a*fps-1e-6)/fps, math.ceil(b*fps-1e-6)/fps
        # Measure only each phrase, away from the intentional retained padding.
        phrase = next((x, y) for x, y in [(1,3),(4,6),(7,9)] if a <= x < b)
        start = offset + phrase[0] - a
        levels.append(db(output_audio[int((start+.5)*rate):int((start+1.)*rate)]))
        gains.append(db(output_audio[int((start+1.7)*rate):int((start+1.95)*rate)]) - levels[-1])
        # 30 ms after phrase onset should already match its steady audible level.
        onset = db(output_audio[int((start+.03)*rate):int((start+.10)*rate)])
        assert abs(onset-levels[-1]) < .5, (onset, levels[-1])
        offset += b-a
    assert max(levels)-min(levels) < 1.5, levels
    assert max(abs(g) for g in gains) < .25, gains
    report.append({'test': 'silence-and-level', 'level_spread_db': max(levels)-min(levels),
                   'max_pre_cut_change_db': max(abs(g) for g in gains)})

    # Silent video-only inputs and every linked audio track stay supported.
    noaudio = p / 'no-audio.mp4'
    run('ffmpeg', '-v', 'error', '-i', source, '-an', '-c:v', 'copy', noaudio)
    project.write_text(json.dumps({'clips': [{'path': str(noaudio), 'end': 3}]}))
    result = run(CLI, '--timeline', project, p / 'compact.mp4', 'compact', env=env)
    assert b'Single pass:' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    assert abs(json.loads(result.stdout)['after_duration'] - 3) < .04
    report.append({'test': 'compact-no-audio', 'single_pass': True})
    multiaudio = p / 'two-tracks.mp4'
    run('ffmpeg', '-v', 'error', '-i', source, '-map', '0:v', '-map', '0:a', '-map', '0:a',
        '-c', 'copy', multiaudio)
    project.write_text(json.dumps({'clips': [dict(c, path=str(multiaudio)) for c in clips]}))
    output = p / 'two-tracks.mkv'
    result = run(CLI, '--timeline', project, output, 'lossless', env=env)
    assert b'Single pass:' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    for track in (0, 1):
        actual = samples(output, track)
        assert len(actual) == len(expected_audio)
        assert max(abs(a-b) for a, b in zip(actual, expected_audio)) < 1e-12
    report.append({'test': 'two-audio-tracks', 'aligned_audio_samples_per_track': len(expected_audio)})

    # Reordering still uses the compatible renderer.
    for name, src, edits in [
        ('reordered', source, [{'path': str(source), 'start': 3, 'end': 4},
                               {'path': str(source), 'start': 1, 'end': 2}]),
    ]:
        project.write_text(json.dumps({'clips': edits}))
        result = run(CLI, '--timeline', project, p / f'{name}.mp4', 'high', env=env)
        assert b'Single pass:' not in result.stderr, result.stderr.decode()
        assert abs(json.loads(result.stdout)['after_duration'] - 2) < .1
        report.append({'test': name, 'compatible_fallback': True})

    # VFR and NTSC now use the timestamp-aware single-session renderer.
    vfr = p / 'variable.mp4'
    run('ffmpeg', '-v', 'error', '-i', source, '-an', '-vf', "select='not(mod(n,7)) + gt(mod(n,11),1)'",
        '-fps_mode', 'vfr', '-c:v', 'libx264', '-preset', 'veryfast', vfr)
    ntsc = p / 'ntsc.mp4'
    run('ffmpeg', '-v', 'error', '-i', source, '-vf', 'fps=30000/1001', '-c:v', 'libx264',
        '-preset', 'veryfast', '-c:a', 'aac', ntsc)
    for name, src in [('variable', vfr), ('ntsc', ntsc)]:
        project.write_text(json.dumps({'clips': [{'path': str(src)}]}))
        result = run(CLI, '--timeline', project, p / f'{name}-out.mp4', 'high', env=env)
        assert b'Single pass:' in result.stderr and b'timestamp-aware=true' in result.stderr, result.stderr.decode()
        assert b'fallback' not in result.stderr, result.stderr.decode()
        report.append({'test': name, 'timestamp_single_pass': True})
    # Exact coarse clocks (including OBS's 1/60) are valid CFR when all packet
    # timestamps pass the grid check. Precision alone must not reject them.
    coarse = p / 'obs-clock.mp4'
    run('ffmpeg', '-v', 'error', '-i', source, '-c', 'copy', '-video_track_timescale', 60, coarse)
    project.write_text(json.dumps({'clips': [dict(c, path=str(coarse)) for c in clips]}))
    output = p / 'obs-clock.mkv'
    result = run(CLI, '--timeline', project, output, 'lossless', env=env)
    assert b'timestamp-aware=false' in result.stderr and b'fallback' not in result.stderr, result.stderr.decode()
    assert hashes(output) == expected_frames
    actual = samples(output)
    assert len(actual) == len(expected_audio)
    assert max(abs(a-b) for a,b in zip(actual,expected_audio)) < 1e-12
    report.append({'test': 'coarse-OBS-clock', 'cfr_single_pass': True, 'exact_samples': len(actual)})
    assert not list(p.glob('.quietcut-*')), 'Export workspace was left behind'
    print(json.dumps({'passed': True, 'results': report}, indent=2))
