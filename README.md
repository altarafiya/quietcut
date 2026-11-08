![QuietCut banner with a silver video timeline](docs/media/quietcut-banner.svg)

# QuietCut

Trim and reorder video clips, shorten pauses, and level dialogue in a desktop
editor that processes files locally. QuietCut uses a Rust media engine and FFmpeg.

[Download for Linux · amd64 .deb](https://github.com/altarafiya/quietcut/releases/latest) ·
[Build from source](#build-from-source) ·
[Windows 1.8 notes](docs/windows-1.8.md)

## Edit on the timeline

![QuietCut on Linux displaying the Blue Hour demo with three clips and silence markers](docs/media/editor.png)

Import or drop videos, scrub the ruler, and drag clip edges to trim. Move clips
on the track, split at the playhead, or undo an edit. The red strips mark
detected pauses that QuietCut will skip when silence removal is enabled.

## Preview and export

![QuietCut's Export window showing the Balanced preset, quality controls, and before and after durations](docs/media/export.png)

Pick a Gentle, Balanced, Tight, or Custom silence preset. **Even audio** applies
compression and peak limiting. Export as a fast stream copy, lossless video,
or high-quality or compact H.264. QuietCut uses NVIDIA encoding when supported
by FFmpeg and the installed driver.

**Fast copy** is the quickest choice for compatible clips. It keeps the video
stream, cuts at keyframes, and turns off silence removal. **Lossless** preserves
the retained frames pixel for pixel. The two H.264 settings trade file size
against detail.

Screenshots show QuietCut running on Linux with the illustrated Blue Hour demo.

## Install on Linux

Download the amd64 package from [Releases](https://github.com/altarafiya/quietcut/releases)
and install it through APT. It was tested on PikaOS 4 and needs glibc 2.39 or newer,
FFmpeg, and the Qt 6 Python multimedia bindings from your distribution.

```sh
sudo apt install ./quietcut_1.8.0-4_amd64.deb
```

Open **QuietCut** from the application menu, or run `quietcut`.

## Build from source

- **Linux** Install Rust and Cargo, a C compiler, binutils, and the Debian
  packaging tools. Run `sh linux/build-deb.sh ./dist`. See the
  [Linux notes](linux/README.md) for playback and export behavior.
- **Windows** Build with the Rust MSVC toolchain and the Windows SDK.
  See the [Windows 1.8 notes](docs/windows-1.8.md).

QuietCut is available under the [MIT license](LICENSE). See the
[third-party notices](THIRD-PARTY-NOTICES.md) for dependency licenses.
