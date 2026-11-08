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

## Build the installer

Install Rust and Cargo, a C compiler, binutils and the Debian packaging tools.
From the repository root, run `sh linux/build-deb.sh ./dist`. Install the
result with `sudo apt install ./dist/quietcut_1.8.0-4_amd64.deb` on x86-64.
The package declares its FFmpeg, Qt and Python dependencies for APT.
