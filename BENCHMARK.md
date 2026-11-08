# QuietCut 1.8 — export startup fix and performance

The reported input was `Videos/2026-10-24 11-51-48.mp4`: a 13:30.333 OBS recording, 1920 × 1080, nominally 60 fps, with 48 kHz stereo AAC. Balanced detects 115 retained ranges. Its packet timestamps contain a missing frame position, so their real timing must be preserved instead of forcing CFR from the nominal frame-rate metadata.

## Reported 18% stall

The 1.7 audio graph reproduced the failure to produce output within 30 seconds on the full recording; the user observed it waiting at 18% for a minute. A 90-second excerpt did not expose the long-file problem. Separate audio/video demuxers and switching to CPU decoding did not resolve it.

Version 1.8 removes independent discard sinks from the audio graph. Retained sections and empty discarded sections pass through the same ordered output. Empty sections contribute no samples and no duration. This avoids draining future audio sections while the video output waits. CPU/GPU sample-accuracy tests verify the result across 140 cuts, including NTSC, VFR, delayed audio and multiple tracks.

The packaged 1.8 GUI exported the complete reported recording successfully:

| Check | Result |
|---|---:|
| First visible rendering progress, including analysis and GPU setup | 7.871 s |
| First encoded progress after FFmpeg launch | 1.597 s |
| Whole native GUI export and completion verification | 138.136 s |
| Retained frames / ranges | 44,287 / 115 |
| Output length | 738.133 s |
| Output size | 46,736,941 bytes |

The native test enabled Balanced silence removal, Even audio and High quality, checked the actual saved file and the export popup, requested the completion sound and verified completion buttons. The source SHA-256 was unchanged after export.

Frame count and output timestamps are checked against the retained source frames. Codec/container duration rounding can make the video stream end one frame before the audio; the output container retains the full selected audio duration.

## Same-quality rendering comparison

On this PC's GTX 1650, using the previous complete 625.469-second variable-timing recording with Balanced, Even audio and High quality:

| Version | Whole export | Output length | Output size |
|---|---:|---:|---:|
| 1.7 | 151.374 s | 604.799 s | 47,629,086 bytes |
| 1.8 | 109.518 s | 604.799 s | 48,038,714 bytes |

Version 1.8 took **27.7% less time** (1.38× throughput), saving 41.856 seconds. The encoded video and audio payloads have identical SHA-256 hashes between these two outputs. The MP4 files have different chunk/interleaving metadata and sizes. The comparison ran sequentially without another encoding or compilation job.

Video payload SHA-256: `5cb1324cbd80dc3dac2c68ad91303b8b1619080f36e9ee481c93cce5b774c2e2`

Audio payload SHA-256: `1aa078d49469c56bd580443a6bc3aa119d7292b96336091f2e4c7512e8b6ce00`

NVENC P4, HQ tuning, CQ 18, spatial AQ, resolution and the causal leveling filter are unchanged. AAC remains stereo 48 kHz at 192 kbit/s. Retained pictures and their timestamps are unchanged. Results vary with source content, cut count and GPU load.

## Progress and recovery

Progress now uses encoded frame count as well as muxed output time, so audio/container buffering does not hide active encoding. If the fast encoder produces neither new frames nor advancing output time for 20 seconds, it is stopped and export retries the compatible renderer. Cancellation remains available during startup and rendering.

Exact coarse CFR clocks, including 1/60, can use the CFR renderer when every packet passes the full frame-grid check. Missing/duplicate frame positions still reject that path and preserve source timing through the variable-rate renderer.

Seventeen Rust unit tests and Clippy passed, including tests for monotonic encoder heartbeat and termination of a stalled child without cancelling fallback. Media regressions cover CPU/GPU pixel and sample preservation, quiet speech, encoded audio leveling, stream copy, mixed sources, export workflow and cancellation.

Version 1.6 is preserved as a working fallback. Version 1.8 uses separate preferences and a separate application folder.
