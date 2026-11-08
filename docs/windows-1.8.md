# QuietCut 1.8

A native Windows video editor, built in Rust with the Windows API. No browser, Electron, Python runtime, account, or upload.

## Version 1.8

Version 1.8 fixes the long startup at 18% seen in 1.7 on a full OBS recording with many silence cuts. Preferences are stored separately in `settings-v18.json`, initially reading 1.7 preferences, with 1.6 as a fallback. Version 1.6 remains available as a working fallback. The desktop QuietCut shortcut opens 1.8; QuietCut 1.6 opens the preserved release.

The variable-timing renderer now sends retained and empty audio sections through one ordered output. Independent discard sinks could drain future audio sections during startup, delaying video output on a long recording. Empty sections contribute no samples or time; CPU/GPU tests compare the complete selected audio against the decoded source. Coarse constant-rate clocks, such as 1/60 second, can also use the CFR renderer when every packet passes the existing frame-grid validation. Videos with missing frame positions keep their original timing through the variable-rate path.

Progress follows both encoded frame count and output timestamps, so buffering in the final container does not hide active rendering. If the fast renderer produces neither new frames nor advancing output time for 20 seconds, QuietCut stops that process and retries the compatible renderer. Cancellation still stops the job immediately. NVENC P4, CQ 18/23, spatial AQ, resolution, source-frame selection and audio leveling settings are unchanged.

The single-session GPU renderer supports variable frame timing, NTSC rates, coarse container clocks, nonzero start times and missing frame-count metadata. Export reads actual source timestamps, retains selected frames and their timing, and routes linked audio through sample-exact cuts in one FFmpeg process. See BENCHMARK.md for measured results and limits.

- File name can be edited directly in Export. The extension follows the quality mode. Existing names receive a numeric suffix instead of being overwritten.
- Export shows an animated preparation indicator, progress percentage, elapsed time and an approximate remaining time once measurable progress is available.
- A successful export requests the standard Windows completion chime and reveals Open folder and New project buttons with icons.
- New project is also in the main header (Ctrl+N). It asks for confirmation with No selected by default, then clears the timeline, undo history, preview and previous export state. Source videos and exported files remain on disk.

The 1.6 workflow controls above remain available. Progress display updates at most four times per second between engine updates.

Compatible videos export through one decoder and one encoder session, with all silence cuts applied together. CUDA frames stay on the GPU for NVENC export. Audio is encoded directly into the final output, avoiding a PCM intermediate for every section and the final segment-joining pass. NVENC P4, CQ 18/23, CPU CRF 18/23 and lossless encoder settings are unchanged.

Cut selection uses a balanced decision tree, keeping expression depth and per-frame evaluation work small even with hundreds of pauses. Source packet timestamps are checked before enabling this path: an average frame rate is insufficient to prove constant frame timing. CFR audio and video share integer frame boundaries; other supported inputs use a clock shared with audio samples. Half-open cut boundaries keep source frames whose timestamps fall inside each retained interval. The last fractional audio-only tail is limited to the final video frame in this path.

The fast renderer supports ordered cuts from one source in High, Compact and compatible Lossless modes. Integral CFR inputs retain the existing frame-grid path. Other supported progressive H.264, HEVC, VP9, MPEG-2 and VC-1 inputs use their actual packet presentation times, checked for missing or duplicate timestamps. Cut edges align to retained source pictures; audio edges round once to the nearest sample. A shared video/audio clock prevents cumulative drift. Output frame count, dimensions, audio-track count and duration are verified before publication. Lossless additionally requires 48 kHz source audio. Different sources, reordered clips, interlaced variable-timing inputs, high-bit-depth video, long discarded heads, very sparse selections, untrustworthy timestamps and exceptionally large packet indexes retain the compatible segmented renderer. Manual multi-clip leveling also keeps the per-clip compressor reset path. An FFmpeg or driver failure retries with the compatible renderer; cancellation still stops the export.

For one-source silence removal with Even audio, the causal compressor processes original pauses before those pauses are discarded. This lets gain recover between phrases without future-window attenuation. The regression suite checks audible onset level, pre-cut level, and quiet/loud consistency on the actual encoded output.

## Run

Open `QuietCut.exe`. Your existing FFmpeg installation is detected automatically. Nothing else needs installing on this PC.

On another PC, supply `ffmpeg.exe` and `ffprobe.exe` in a `tools` folder beside the app, or install FFmpeg and add its `bin` folder to PATH. `QUIETCUT_FFMPEG` can also point to that folder. FFmpeg is a separate dependency and is not bundled in this download.

Windows 10/11, x64. The Microsoft C runtime is linked statically.

## Edit

- **Import** accepts multiple videos. Dropping files adds them to the end of the timeline.
- Click the timeline ruler to seek. **Play** previews the edited sequence with linked audio. Silence removal and Even audio are controlled by their icon toggles in Export and apply live during preview. Silence analysis runs in the background; removed pauses appear along the timeline.
- The header contains New project, Import, Export and the theme button. Zoom controls share one footer row; Export is no longer stacked underneath them.
- Maximize or resize the window; the preview, transport, timeline and export controls follow the available space. The native maximize/restore resize event is handled correctly. The interface uses a neutral charcoal/silver theme with outlined selections.
- Select a clip and drag its left or right edge to trim it.
- **Split** cuts at the playhead. Split twice, select the middle clip, and **Delete** to remove a section.
- Drag clips to reorder them, or use **Left / Right**.
- **Undo** restores earlier edits, including trim, split, delete, import, and reorder.
- **+ / − / Fit** zoom the timeline. The horizontal scrollbar navigates longer timelines. Shift + mouse wheel scrolls; mouse wheel zooms.

Shortcuts: **Ctrl+N** new project, **Ctrl+O** import, **Ctrl+B** split, **Delete** remove selected clip, **Ctrl+Z** undo, **Space** play/pause, **← / →** step one source frame, **Home / End** seek to the ends.

## Export

Silence removal and audio leveling are optional export operations. They do not change your editing timeline.

**Remove silence:** Choose the Gentle, Balanced, Tight, or Custom icon. Only Custom shows numerical sliders. The presets use a relative RMS threshold based on the recording’s typical speech level, with native WebRTC voice detection protecting quiet words and edges. Tight shortens more pauses without raising the silence threshold. Bounded detector-only gain and a measured noise floor prevent faint codec tails from being mistaken for words. This also supports recordings with low overall volume. Detection analyzes the original audio in one pass; only the voice classifier receives temporary gain. The output audio is unaffected unless Even audio is enabled. Results are cached for preview and export. All channels and audio tracks must be quiet before a silent range is removed. Completely silent clips are skipped; if everything would be removed, export stops without creating an empty video. Clips with no audio remain in the timeline.

**Exact silence cuts:** Silence removal automatically selects High quality when the previous quality was Original / Fast copy. Fast copy is disabled while silence removal is on: a video with five-second keyframe spacing cannot lose its short pauses through copying alone. Lossless remains available to preserve decoded pixels. With silence removal off, Fast copy is available for the fastest compatible joins.

**Even audio:** One toggle enables fast, causal RMS compression and a peak limiter. The compatible renderer resets gain per section; the fast silence path allows recovery during original pauses before removing them. Neither uses future-window gain smoothing. It changes audio intentionally; keep it off to preserve original levels. The preview uses the same filter. Silence detection remains independent of leveling.

**Save to:** Select Video folder to export beside the first input, or Choose folder to remember a default destination. Edit File name to choose the output name; the default is based on the source. If the name already exists, a numeric suffix is added. Ask each time restores the save dialog.

The export panel has a Preview button, so preset, audio, and quality changes can be checked before export.

| Quality | Behavior |
|---|---|
| Fast copy · silence off | Copies compressed video and audio when silence removal is off. Manual boundaries snap to source keyframes. Joining requires matching formats. |
| Lossless · exact cuts | Keeps retained decoded pixel values. Uses NVIDIA lossless H.264 for supported 8-bit YUV420 sources, otherwise multithreaded FFV1. Audio is preserved as PCM when leveling is off. Files can be large. |
| High quality · smaller | Frame-accurate H.264, NVENC CQ 18 or CPU x264 CRF 18. Audio is encoded to AAC once. |
| Compact · smaller still | NVENC CQ 23 or CPU x264 CRF 23. Smaller files, with more compression. |

NVIDIA acceleration is automatic. The GTX 1650 on this PC passed the runtime NVENC test. Supported sources use CUDA decoding, CUDA padding, and NVENC encoding without downloading decoded video frames to the CPU. Other input formats can use CPU decoding with NVENC encoding. Failed GPU initialization retries on the CPU. Higher bit depths retain a CPU path rather than being silently reduced to 8-bit for this GPU.

NVENC P4 uses hardware encoding with high-quality tuning; CPU encoders use FFmpeg's automatic thread count. No particular language or GPU guarantees the fastest result for every file. Fast copy mode avoids encoding entirely. Quality-based settings cannot guarantee that every input becomes smaller, especially already compressed videos.

Before → After cards in the export window show the timeline size and length, estimated retained length after analysis, and actual output size and duration after export. The window stays open during export with percentage, elapsed time and an estimated remaining time. Preparation shows an animated indicator until there is enough measured progress to estimate a rate. The estimate adapts to measured throughput and may change between phases. After success it displays actual output statistics, plays the Windows notification sound, and shows Open folder and New project buttons. Cancel stops an active job; closing the window during export cancels it and cleans up before closing. Compressed output size is only known after encoding. The timeline's Before size is an estimate proportional to the retained source durations. Smaller sources are centered in a canvas large enough for the imported videos; they are not stretched. Mixed sources use stereo 48 kHz audio in the compressed modes and add silence for missing tracks. Lossless mixed-source export requires matching source pixel formats and audio layouts. Color tags are forwarded; complex mixed HDR workflows, alternate video streams, subtitles, chapters, attachments, and interactive metadata are outside this single-video-track editor. The source files are never modified or overwritten.

Video preview uses Windows' native Media Engine; audio-only FFmpeg decoding feeds native WASAPI with a small buffer. The next kept section is prefetched, and the audio playback clock keeps video synchronized. No temporary preview video is encoded. Playback previews the first audio track as stereo; export retains the supported audio tracks. Available preview codecs depend on Windows and installed codec support. A file that Windows cannot preview may still be edited and exported through FFmpeg. The timeline supports one video track with its linked audio; it is not a multi-layer compositor. Edits live in the current session; save the exported video before closing.

## Open-source implementation choices

The heavy processing reuses native open-source engines rather than implementing codec or DSP loops in the GUI:

- [Native WebRTC VAD](https://github.com/kaegi/webrtc-vad), backed by libfvad, for low-cost speech protection in presets; [FFmpeg silencedetect](https://ffmpeg.org/ffmpeg-filters.html#silencedetect) for manually configured Custom detection.
- [FFmpeg segment and concat muxers](https://ffmpeg.org/ffmpeg-formats.html) for stream copying and joining.
- FFmpeg select/aselect, settb/setpts, asetnsamples and aresample for single-pass frame and sample selection. This directly reuses FFmpeg's native filters; there is no Python media processing in the application.
- [FFmpeg asegment](https://ffmpeg.org/ffmpeg-filters.html#segment_002c-asegment) routes consecutive audio intervals at exact sample boundaries. Ordered audio-only concat receives both kept sections and empty discarded sections through one output, avoiding independent sinks which eagerly request future audio. Timestamp offsets use balanced expressions, preserving original VFR intervals within each retained section.
- [FFmpeg acompressor](https://ffmpeg.org/ffmpeg-filters.html#acompressor) and alimiter for causal volume leveling and peak limiting.
- [FFmpeg NVENC](https://ffmpeg.org/ffmpeg-codecs.html) and [NVIDIA's FFmpeg integration](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/ffmpeg-with-nvidia-gpu/index.html) for GPU decoding and encoding.
- FFmpeg FFV1 and x264 for CPU lossless and quality-based compression.

[LosslessCut](https://github.com/mifi/lossless-cut) and [Auto-Editor](https://github.com/WyattBlue/auto-editor) were also reviewed. QuietCut calls the media engine directly and provides a compact native interface. No claim is made that those applications were benchmarked or that this app is universally faster.

## Build

Install Rust's x64 MSVC toolchain and Windows C++ Build Tools. The Windows SDK resource compiler embeds the icon and version information when available.

```powershell
cargo build --release --bin quietcut
cargo test --lib
```

The `quietcut-cli` binary is included in the source for repeatable media tests. `QUIETCUT_DISABLE_GPU=1` forces CPU rendering. `QUIETCUT_FORCE_LEGACY=1` selects the segmented renderer for comparisons. `QUIETCUT_PROFILE=1` writes single-pass eligibility/execution diagnostics to stderr without changing CLI JSON output.

## Verification on this PC

See `BENCHMARK.md` for the 1.8 startup regression check on the reported OBS file and the comparison with 1.7 on the previous variable-timing benchmark. Inputs that already used the CFR fast path retain its encoder settings.

- Seventeen unit tests cover clip mapping, split bounds, reorder, silence padding, keyframe cuts, path escaping, invalid settings, preview seeking across removed pauses, previous preference compatibility, exact-cut quality selection, frame/sample boundaries, expression depth with 4,000 cuts, valid output names, collision protection, progress estimation, actual encoder heartbeat and termination of a stalled child without cancelling fallback.
- CPU and GTX 1650 lossless exports passed frame-by-frame hash comparisons and exact audio sample comparisons across 140 non-keyframe cuts. The timestamp suite covers VFR, NTSC, coarse clocks without frame counts, nonzero start times and delayed audio, plus linked audio tracks and video-only input. Matroska output retains its native millisecond timestamp precision; MP4 uses the shared high-resolution clock.
- A mixed-resolution/frame-rate timeline, including a clip with no audio, exported successfully to one synchronized video.
- Tight retained all 299 active speech frames in a synthesized voice recording peaking at −52.7 dBFS while removing silent pauses. The check also verifies removable pauses and the causal audio gain envelope. Actual speech and noise vary; no automatic detector guarantees every recording.
- The quiet/loud envelope tests leveled an 18.84 dB input difference to about 0.94 dB. The encoded NTSC test's largest pre-cut level change was about 0.0014 dB, and its onset check passed.
- Native initialization, shutdown, WASAPI buffering and full playback drain passed hidden tests. The app rendered its own normal, enlarged, and export-panel layouts for visual inspection. The native GUI checks also exercise real Windows maximize/restore commands and native resize events at several sizes, asserting control bounds and anchors. An end-to-end native export-window check verifies final statistics against the real file; another checks closing during export cancels the job and cleans up. Real speech listening and manual timeline interaction were not repeated in this revision.

Run `python tests/timestamp_export.py`, `python tests/fast_export.py`, `python tests/media_pipeline.py` and `python tests/audio_regression.py` after a release build. The suites check 140 non-keyframe cuts against source frame hashes and exact decoded audio samples on CPU and GPU; VFR/NTSC timing, coarse clocks, nonzero start times, delayed audio, multiple audio tracks, video-only export, the encoded leveling envelope, quiet speech, mixed-source joins and reordering. Python is needed only for the test scripts, never for the app. The CLI reuses one media probe per source within a project, matching the GUI's import behavior.

Native workflow checks: --project-test INPUT OUTPUT_FOLDER checks real Yes/No confirmation paths from both windows; --export-window-test INPUT OUTPUT_FOLDER edits a Unicode file name and verifies existing-file protection and completion controls; --export-cancel-test LONG_INPUT OUTPUT_FOLDER checks live progress/ETA and cancellation cleanup. --layout-test and --render-test cover native sizing and visual states. Test flags do not save application preferences.

Append `silence` to either export test to enable Balanced silence cuts. Export tests assert that real rendering progress begins within 15 seconds, and the completion test checks the real saved file and popup results.
