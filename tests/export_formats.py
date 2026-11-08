"""Integration checks for the Linux HEVC and audio-only project exports."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

CLI = Path(__file__).resolve().parents[1] / "target/release/quietcut-cli"


def run(*args, env=None, check=True):
    return subprocess.run([str(arg) for arg in args], check=check, text=True,
                          stdin=subprocess.DEVNULL, capture_output=True, env=env)


def probe(file):
    return json.loads(run("ffprobe", "-v", "error", "-show_streams", "-show_format",
                          "-of", "json", file).stdout)


with tempfile.TemporaryDirectory(prefix="quietcut-export-formats-") as path:
    root = Path(path)
    video = root / "recording.mp4"
    silent = root / "video-only.mp4"
    run("ffmpeg", "-v", "error", "-f", "lavfi", "-i",
        "testsrc2=size=640x360:rate=30:duration=6",
        "-f", "lavfi", "-i", "aevalsrc=if(lt(mod(t\\,3)\\,1.5)\\,0.2*sin(2*PI*440*t)\\,0):s=48000:d=6",
        "-f", "lavfi", "-i", "sine=frequency=800:sample_rate=48000:duration=6",
        "-map", "0:v", "-map", "1:a", "-map", "2:a", "-c:v", "libx264", "-preset", "fast",
        "-crf", "13", "-c:a", "aac", video)
    run("ffmpeg", "-v", "error", "-f", "lavfi", "-i",
        "testsrc2=size=640x360:rate=30:duration=1", "-c:v", "libx264", silent)
    project = root / "project.json"

    def export(name, clips, *, audio=False, quality=4, compression=24, legacy=False, silence=False, check=True):
        project.write_text(json.dumps({
            "clips": clips,
            "audio_only": audio,
            "remove_silence": silence,
            "settings": {"threshold": -50, "min_silence": .5, "padding": .14, "quality": quality,
                         "compression_quality": compression, "level": False, "dark": True},
        }), encoding="utf-8")
        output = root / name
        env = dict(os.environ, QUIETCUT_DISABLE_GPU="1")
        if legacy:
            env["QUIETCUT_FORCE_LEGACY"] = "1"
        result = run(CLI, "--render-project", project, output, env=env, check=False)
        if check:
            assert result.returncode == 0, result.stderr
            return output, json.loads(result.stdout.splitlines()[-1])
        return result

    clips = [{"path": str(video)}]
    detailed, _ = export("detailed.mp4", clips, compression=19)
    small, _ = export("small.mp4", clips, compression=30)
    legacy, _ = export("legacy.mp4", clips, legacy=True)
    for file in (detailed, small, legacy):
        stream = probe(file)["streams"][0]
        assert stream["codec_name"] == "hevc" and stream["codec_tag_string"] == "hvc1"
        assert abs(float(stream["duration"]) - 6) < .08
    assert small.stat().st_size < detailed.stat().st_size

    track, report = export("trimmed.m4a", [
        {"path": str(video), "start": 1.13, "end": 2.51},
        {"path": str(silent)},
        {"path": str(video), "start": 3.04, "end": 4.68},
    ], audio=True, quality=0)
    streams = probe(track)["streams"]
    assert len(streams) == 2 and all(s["codec_type"] == "audio" and s["codec_name"] == "aac" for s in streams)
    assert abs(report["duration"] - 4.02) < .06
    one_track = root / "pauses.mp4"
    run("ffmpeg", "-v", "error", "-i", video, "-map", "0:v", "-map", "0:a:0", "-c", "copy", one_track)
    cleaned, cleaned_report = export("without-pauses.m4a", [{"path": str(one_track)}],
                                      audio=True, silence=True)
    assert len(probe(cleaned)["streams"]) == 1
    assert 2.9 < cleaned_report["duration"] < 4.0
    same_file = track.read_bytes()
    result = export(track.name, clips, audio=True, check=False)
    assert result.returncode != 0 and track.read_bytes() == same_file
    result = export("no-audio.m4a", [{"path": str(silent)}], audio=True, check=False)
    assert result.returncode != 0 and "no audio tracks" in result.stderr
    print(f"Verified HEVC quality ({detailed.stat().st_size} to {small.stat().st_size} bytes), "
          "compatible HEVC rendering, two-track audio with exact edits, and output safety.")
