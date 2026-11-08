# QuietCut on Linux

Open **QuietCut** from the application menu. Import or drop videos, then scrub
the time ruler. Drag a clip by its body to reorder it, or pull an edge to trim.
Use Split at the playhead and Undo to reverse an edit.

**Export** opens a separate window for silence presets, audio leveling, quality
and the output folder. Red strips on the timeline indicate detected pauses.
The timeline player skips these pauses when silence removal is enabled. To
hear the leveling filter, enable **Even audio** and select **Preview** in the
Export window. QuietCut renders that preview to a temporary file and deletes it
when you change the edit or close the app.

The native Rust CLI is available as `quietcut-cli`. FFmpeg and FFprobe are
system dependencies. NVIDIA acceleration is used when FFmpeg and the installed
driver support it. The editor works with CPU encoding as well.

## Compression and audio

Choose **Smaller file · HEVC** for adjustable H.265 compression. Start at
**24** and move toward **More detail** or **Smaller file** as needed. **Preview
sample** encodes up to eight seconds to check your settings. The control uses
CRF for software x265 and CQ for NVIDIA encoding, so results can differ.

Choose **Audio only · M4A** to export the edited audio without encoding video.
It preserves linked tracks as stereo AAC at 192 kb/s per track and applies
silence removal and **Even audio** when enabled.

## Build the installer

Install Rust and Cargo, a C compiler, binutils and the Debian packaging tools.
From the repository root, run `sh linux/build-deb.sh ./dist`. Install the
result with `sudo apt install ./dist/quietcut_1.8.0-5_amd64.deb` on x86-64.
The package declares its FFmpeg, Qt and Python dependencies for APT.
