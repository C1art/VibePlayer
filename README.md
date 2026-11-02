# VibePlayer

A light, fast video player for Windows 11 with a bare-bones lossless trimmer.

- **Plays almost anything** FFmpeg can open: MP4, MKV, WebM, MOV, AVI, TS and more, with
  hardware decoding (falling back to software automatically), HDR tone mapping, deinterlacing,
  embedded subtitles, multiple audio tracks, playback speed from 0.25× to 4× without pitch
  change, frame stepping both ways, and resume-where-you-left-off.
- **Trims without re-encoding.** Press `E`, drag the handles (or press `I` / `O`), and export.
  Video packets are copied byte-for-byte; the editor shows where the cut really starts, since a
  copied video can only begin on a keyframe. Changing volume or the 3-band EQ re-encodes the
  audio only.
- **Stays out of the way.** Controls fade while playing; everything has a shortcut (`F1`).

## Install

Download `VibePlayer-x.y.z-setup.exe` from
[Releases](https://github.com/c1art/vibeplayer/releases) and run it. No admin rights needed.
To make VibePlayer open your videos, choose it in **Settings → Apps → Default apps**.
VibePlayer checks for updates once a day and asks before installing anything.

## Shortcuts

| Keys | Action |
|---|---|
| Space / K | Play or pause |
| ← / → | Back / forward 5 s (Shift: 1 s, Ctrl: 1 min) |
| J / L | Back / forward 10 s |
| , / . | Previous / next frame |
| ↑ / ↓, mouse wheel | Volume |
| M | Mute |
| [ / ] / Backspace | Slower / faster / normal speed |
| 0 – 9 | Jump to 0 % – 90 % |
| A / S | Next audio / subtitle track |
| F, double-click | Fullscreen |
| Tab | File info |
| E | Trim editor; I / O set start / end |
| Ctrl + O / Ctrl + W | Open / close file |
| Ctrl + , | Settings |

## Build

Requirements: Rust (stable), Visual Studio C++ build tools, LLVM (for bindgen), a shared
FFmpeg 8.1+ build (for example BtbN's `win64-lgpl-shared`), and Inno Setup 6 for the installer.

```powershell
$env:FFMPEG_DIR = "C:\ffmpeg"          # contains bin\, include\, lib\
$env:PATH = "$env:FFMPEG_DIR\bin;$env:PATH"
cargo run -p app -- "C:\path\to\video.mkv"
cargo build --release
iscc installer\vibeplayer.iss          # writes target\installer\VibePlayer-x.y.z-setup.exe
```

Tests: `cargo test --workspace`; decoding and export tests against `tests/fixtures/` run with
`cargo test -p media -p export -p playback -- --ignored`.

## Releasing

1. Bump `version` in the root `Cargo.toml`.
2. Tag the commit `vX.Y.Z` and push the tag.
3. The Windows workflow (`installer/windows-ci.yml`, to be moved to `.github/workflows/`)
   builds the installer and attaches it to the GitHub release. The in-app updater looks for
   the newest non-prerelease whose asset ends in `-setup.exe`.

## License

VibePlayer is free software under the [GNU General Public License v3.0](LICENSE).
FFmpeg is used as shared libraries under the LGPL; both licenses are installed next to the app.
