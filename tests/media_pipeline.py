"""Synthetic real-media checks. Requires a release build and FFmpeg on PATH."""
import array
import collections
import json
import math
import os
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
CLI = ROOT / ("target/release/quietcut-cli.exe" if os.name == "nt" else "target/release/quietcut-cli")


def run(args, env=None, check=True):
    return subprocess.run([str(a) for a in args], capture_output=True, env=env,
                          check=check, creationflags=0x08000000 if os.name == "nt" else 0)


def frame_hashes(path):
    data = run(["ffmpeg", "-v", "error", "-i", path, "-map", "0:v:0", "-f", "framemd5", "-"]).stdout.decode()
    return collections.Counter(line.split(",")[-1].strip() for line in data.splitlines() if not line.startswith("#"))


with tempfile.TemporaryDirectory(prefix="quietcut-media-test-") as folder:
    p = pathlib.Path(folder)
    source = p / "视频 it's.mkv"
    expression = "if(between(t\\,1\\,3)\\,0.08*sin(2*PI*440*t)\\,if(between(t\\,6\\,8)\\,0.7*sin(2*PI*440*t)\\,if(between(t\\,10\\,12)\\,0.15*sin(2*PI*440*t)\\,0)))"
    run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=14",
         "-f", "lavfi", "-i", f"aevalsrc={expression}:s=48000:d=14", "-c:v", "libx264", "-preset", "fast",
         "-crf", "16", "-g", "30", "-keyint_min", "30", "-sc_threshold", "0", "-c:a", "pcm_f64le", source])
    mixed = p / "mixed-source.mp4"
    run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=2",
         "-c:v", "libx264", "-preset", "veryfast", mixed])
    silent = p / "silent.mkv"
    run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "color=size=320x180:duration=2",
         "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-t", "2", "-c:v", "libx264", "-c:a", "pcm_s16le", silent])
    source_hashes = frame_hashes(source)
    results = []

    def export(name, clips, mode, level=False, silence=False, gpu=True):
        project = p / f"{name}.json"
        project.write_text(json.dumps({"clips": clips}, ensure_ascii=False), encoding="utf-8")
        output = p / (name + (".mp4" if mode in ("high", "compact") else ".mkv"))
        env = os.environ.copy()
        if not gpu:
            env["QUIETCUT_DISABLE_GPU"] = "1"
        result = run([CLI, "--timeline", project, output, mode, "level" if level else "off", "silence" if silence else "off"], env=env)
        info = json.loads(result.stdout)
        run(["ffmpeg", "-v", "error", "-xerror", "-i", output, "-f", "null", "-"])
        results.append(dict(test=name, **info))
        return output, info

    edits = [{"path": str(source), "start": 1, "end": 3}, {"path": str(source), "start": 6, "end": 8}]
    for name, mode, gpu in [("copy-edits", "copy", True), ("gpu-lossless", "lossless", True), ("cpu-lossless", "lossless", False)]:
        out, info = export(name, edits, mode, gpu=gpu)
        assert abs(info["after_duration"] - 4) < 0.1, info
        assert not (frame_hashes(out) - source_hashes), f"Changed or duplicated pixels: {name}"

    out, info = export("copy-full", [{"path": str(source)}], "copy")
    assert abs(info["after_duration"] - 14) < 0.1
    assert frame_hashes(out) == source_hashes
    out, info = export("mixed", [edits[0], {"path": str(mixed)}], "high", level=True)
    assert abs(info["after_duration"] - 4) < 0.1
    out, info = export("leveled", [{"path": str(source)}], "high", level=True, silence=True)
    plan = json.loads(run([CLI, "--analyze", source, "Balanced"]).stdout)
    assert abs(info["after_duration"] - sum(b-a for a, b in plan["keep"])) < .15, info
    samples = array.array("d", run(["ffmpeg", "-v", "error", "-i", out, "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f64le", "-"]).stdout)
    levels = []
    offset = 0.
    centers = []
    for a, b in plan["keep"]:
        centers.append(offset + (b-a)/2)
        offset += b-a
    for t in centers:
        window = samples[int((t - .3) * 48000):int((t + .3) * 48000)]
        levels.append(20 * math.log10((sum(x*x for x in window) / len(window))**.5))
    assert max(levels) - min(levels) < 1.5, levels

    # Long GOP recordings reproduce the reported no-op: short pauses cannot be
    # removed by compressed stream copying, but exact GPU cuts must remove them.
    long_gop = p / "long-gop.mkv"
    run(["ffmpeg", "-v", "error", "-i", source, "-c:v", "libx264", "-preset", "veryfast",
         "-g", "300", "-keyint_min", "300", "-sc_threshold", "0", "-c:a", "copy", long_gop])
    exact = json.loads(run([CLI, "--analyze", long_gop, "Balanced"]).stdout)
    copied = json.loads(run([CLI, "--analyze", long_gop, "Balanced", "copy"]).stdout)
    assert exact["duration"] < copied["duration"] - 2, (exact, copied)
    _, info = export("long-gop-exact", [{"path": str(long_gop)}], "high", silence=True)
    assert abs(info["after_duration"] - exact["duration"]) < .15, info

    # A wholly silent clip is skipped when other content remains.
    _, info = export("skip-silent", [edits[0], {"path": str(silent)}], "high", silence=True)
    assert abs(info["after_duration"] - 2) < 0.1
    project = p / "empty.json"
    project.write_text(json.dumps({"clips": [{"path": str(silent)}]}))
    empty = p / "must-not-exist.mp4"
    failure = run([CLI, "--timeline", project, empty, "high", "off", "silence"], check=False)
    assert failure.returncode and not empty.exists()
    # Existing output is protected, and all per-job workspaces are cleaned up.
    failure = run([CLI, "--timeline", p / "copy-full.json", p / "copy-full.mkv", "copy"], check=False)
    assert failure.returncode and b"already exists" in failure.stderr
    assert not list(p.glob(".quietcut-*"))
    print(json.dumps({"passed": True, "results": results, "audio_rms_spread_db": round(max(levels)-min(levels), 3)}, indent=2))
