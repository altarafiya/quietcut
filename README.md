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

![QuietCut's Export window showing the Balanced preset, quality controls, and HEVC quality slider and estimated duration](docs/media/export.png)

![Original QuietCut artwork showing an edited film strip branching into a smaller HEVC video and an audio waveform](docs/media/export-art.svg)

- **Smaller file · HEVC** Start at quality **24** and use **Preview sample**
  to check up to eight seconds before encoding the full edit. Lower numbers
  preserve more detail. H.265 playback requires a compatible player.
- **Audio only · M4A** Export your edited audio as AAC, with silence removal
  and audio leveling available. No video is encoded.
- **Fast copy** Keeps the video stream and cuts at keyframes. It turns off
  silence removal. **Lossless** preserves the retained frames pixel for pixel.
  The two H.264 presets trade size against detail and offer wider compatibility.

QuietCut uses NVIDIA encoding when the encoder and driver support it, with
software encoding available as a fallback. File-size savings vary by source
and selected quality.

The [audio-only export window](docs/media/audio.png) keeps the same audio cleanup
controls and hides video quality settings.

Screenshots show QuietCut running on Linux with the illustrated Blue Hour demo.

## Install on Linux

Download the amd64 package from [Releases](https://github.com/altarafiya/quietcut/releases)
and install it through APT. It was tested on PikaOS 4 and needs glibc 2.39 or newer,
FFmpeg, and the Qt 6 Python multimedia bindings from your distribution.

```sh
sudo apt install ./quietcut_1.8.0-5_amd64.deb
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
