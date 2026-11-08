"""Speech retention and gain-envelope checks using real native media processing."""
import array
import json
import math
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CLI = Path(os.environ.get("QUIETCUT_TEST_CLI", str(ROOT / ("target/release/quietcut-cli.exe" if os.name == "nt" else "target/release/quietcut-cli"))))


def run(*args):
    return subprocess.run([str(a) for a in args], capture_output=True, check=True,
                          creationflags=0x08000000 if os.name == "nt" else 0).stdout


def floats(data):
    return array.array("f", data)


def rms(samples, a, b, rate):
    values = samples[int(a * rate):int(b * rate)]
    return 20 * math.log10(math.sqrt(sum(x*x for x in values) / len(values)))


with tempfile.TemporaryDirectory(prefix="quietcut-speech-") as folder:
    p = Path(folder)
    speech = floats(run("ffmpeg", "-v", "error", "-f", "lavfi", "-i",
                        "flite=text='Please keep these quiet words. The next sentence must stay complete.':voice=slt",
                        "-ar", "16000", "-ac", "1", "-f", "f32le", "-"))
    quiet = array.array("f", (x * .003 for x in speech))
    source = array.array("f", (.7 * math.sin(2 * math.pi * 440 * n / 16000) for n in range(16000)))
    source.extend([0.] * 11200)
    speech_start = len(source) / 16000
    source.extend(quiet)
    source.extend([0.] * 16000)
    raw = p / "speech.f32"
    raw.write_bytes(source.tobytes())
    video = p / "speech.mkv"
    run("ffmpeg", "-v", "error", "-f", "lavfi", "-i", "color=size=160x90:rate=30",
        "-f", "f32le", "-ar", "16000", "-ac", "1", "-i", raw, "-shortest",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "pcm_f32le", video)
    begun = time.perf_counter()
    plan = json.loads(run(CLI, "--analyze", video, "Tight"))
    elapsed = time.perf_counter() - begun
    peaks = [max(abs(v) for v in quiet[n:n+160]) for n in range(0, len(quiet)-159, 160)]
    active = [speech_start + n * .01 + .005 for n, peak in enumerate(peaks) if peak > max(peaks) * .04]
    lost = [t for t in active if not any(a <= t <= b for a, b in plan["keep"])]
    assert not lost, f"Quiet speech removed at {lost}; plan={plan}"
    assert sum(b-a for a, b in plan["removed"]) > .65, plan

    # A future loud passage must not turn down the preceding quiet passage.
    rate = 48000
    waveform = array.array("f", ((.08 if n < 2*rate else .7) * math.sin(2*math.pi*440*n/rate)
                                 for n in range(4*rate)))
    raw.write_bytes(waveform.tobytes())
    engine = (ROOT / "src/engine.rs").read_text(encoding="utf-8")
    level_filter = re.search(r'pub const LEVEL_FILTER: &str = "([^"]+)"', engine)[1]
    leveled = floats(run("ffmpeg", "-v", "error", "-f", "f32le", "-ar", rate, "-ac", "1", "-i", raw,
                          "-af", level_filter, "-f", "f32le", "-"))
    pre_cut_change = rms(leveled, 1.7, 1.98, rate) - rms(leveled, .8, 1.2, rate)
    assert abs(pre_cut_change) < .25, f"Pre-cut gain dip: {pre_cut_change} dB"
    spread = abs(rms(leveled, .8, 1.2, rate) - rms(leveled, 2.8, 3.2, rate))
    assert spread < 1.5, f"Leveling failed: {spread} dB"
    print(json.dumps({"passed": True, "quiet_speech_frames_retained": len(active),
                      "speech_peak_dbfs": round(20*math.log10(max(peaks)), 2),
                      "analysis_seconds": round(elapsed, 3), "video_seconds": plan["duration"],
                      "pre_cut_gain_change_db": round(pre_cut_change, 3),
                      "leveled_difference_db": round(spread, 3)}, indent=2))
