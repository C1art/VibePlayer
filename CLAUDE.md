# CLAUDE.md

## Project

VibePlayer: a light Windows 11 video player in Rust with a bare-bones lossless clip trimmer. Its main job is quality playback; its second is fast clip cutting. It is not a video editor.

- **Player** (default): opens straight to the video; controls auto-hide. Play/pause, skip ±, frame step, playback speed, volume/mute, audio track, embedded subtitle track, fullscreen, media keys, HDR, and a file info panel (container, codecs, resolution, frame rate, bitrate, duration, metadata).
- **Editor** (via "Edit"): trim one clip with in/out points (keyframes and the audio waveform shown on the timeline, optional snapping), adjust volume and a basic audio EQ, export. Optional smart cut for a frame-accurate start. Nothing else. EQ exists only here; the player has plain volume.
- **Installer:** Inno Setup installer and uninstaller; registers file types so the user can choose VibePlayer to open videos.
- **Update check:** looks for a newer GitHub release of `c1art/vibeplayer` and offers "Update", "Ask later" or "Skip this version".

Priorities, in order: lightweight (fast startup, low idle CPU and memory), clean modern UI, features. When in doubt, leave it out.

Out of scope: joining files, speed changes in export, playlists, adding or editing subtitles, any other editing.

## Stack

- Rust stable, edition 2024, target `x86_64-pc-windows-msvc`
- UI: `egui` / `eframe`
- GPU: `wgpu` (DX12, Vulkan fallback), WGSL shaders
- Media: FFmpeg (GPL build, for `libx264`/`libx265` in smart cut) via our own safe wrapper over `ffmpeg-sys-next`, including `libavfilter`
- Audio: `cpal` output; resampling, downmix, speed (`atempo`) and EQ run in `libavfilter`
- Concurrency: `crossbeam` channels and queues
- Persistence: `serde` + `serde_json`
- Windows: `windows` crate, `cfg(windows)` only — D3D12/DXGI texture sharing, HDR output, SMTC, taskbar, file associations
- Exe resources: `embed-resource` (build dependency of `app`) — icon and version info
- Installer: Inno Setup 6
- Update check: `ureq` (HTTPS, system trust store via `platform-verifier`) against the GitHub Releases API, `semver` for version comparison
- Ask before adding any dependency not listed here.

## Structure

Cargo workspace. Dependencies flow downward only; `cut` and `library` never depend on `app`, `render`, `media` or `playback`.

- `crates/app/` — eframe entry point, panels, OSD, input, shortcuts, OS integration
- `crates/media/` — FFmpeg wrapper: probe, demux, decode, filter, remux. All FFmpeg `unsafe` lives in `media/src/ffi/`
- `crates/playback/` — engine: clock, decode scheduling, seeking, frame stepping, speed
- `crates/render/` — wgpu presentation, color conversion, HDR; shaders in `render/shaders/*.wgsl`
- `crates/audio/` — cpal output, resampling, volume, downmix, EQ (editor preview only)
- `crates/thumbnails/` — background seek-bar previews with a bounded cache
- `crates/cut/` — cut planning: in/out points, keyframe snapping, timestamp rebasing (pure logic, no I/O); also the shared time types (`Ticks`, `Timebase`, `StreamIndex`)
- `crates/export/` — runs a cut plan through `media`: stream copy, smart-cut head re-encode, audio-only re-encode for volume/EQ, verification
- `crates/update/` — release check, version comparison, ask-later and skipped-version rules, installer download
- `crates/library/` — settings, recents, resume positions, cut sessions, update preferences in `%APPDATA%\VibePlayer`
- `vendor/egui-wgpu/` — egui-wgpu patched for HDR (scRGB) output; see `vendor/README.md` before upgrading egui
- `installer/` — Inno Setup script (`vibeplayer.iss`)
- `.github/workflows/windows-ci.yml` — Windows CI and releases (this agent can't push workflow files; propose changes for the user to commit)
- `tests/fixtures/` — small media files for integration tests; regenerate with `generate.sh`

## Commands

- Run: `cargo run -p app [-- <file>]` (the binary is `vibeplayer.exe`)
- Release: `cargo build --release`
- Installer: `iscc installer/vibeplayer.iss` (after a release build)
- Format: `cargo fmt --all`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Test: `cargo test -p <crate>` or `cargo test --workspace`
- Before building, set `FFMPEG_DIR` to a shared GPL FFmpeg build, 8.1 or newer (BtbN `win64-gpl-shared`); without `libx264`/`libx265` smart cut is unavailable.
- Linux also builds and tests, for development only; it needs `pkg-config` and the ALSA, X11 and FFmpeg dev packages. Without a sound card, run `pulseaudio -D` so audio-clock paths are exercised.
- Release: bump `version` in the root `Cargo.toml` and merge to `main`; CI creates the `vX.Y.Z` tag and release with `VibePlayer-X.Y.Z-setup.exe`, which the updater looks for. Only bump the version when the user asks for a release or agrees to one.

After every change, in order: format, lint, test the changed crate. Test the whole workspace before committing.

## Architecture rules

- **Time:** integer `Ticks` with a rational timebase; `f64` seconds only for display. The player timeline is microseconds from the file's start time; cut points use the source stream's timebase (absolute, so add the file start time when converting).
- **Threads:** the UI thread never decodes, does I/O or waits on a channel. Workers report back over `crossbeam` channels, polled each frame.
- **Audio callback:** no allocation, locks, blocking or logging. Reads only a pre-filled lock-free queue.
- **Clock:** audio is master; video drops or repeats frames to follow. No audio → monotonic system clock. Playback speed uses `atempo`, never pitch-shifting.
- **Seeking:** seek to the prior keyframe, decode and discard to the target. Scrubbing uses keyframes only. A generation counter discards stale frames after each seek.
- **Hardware decode:** on any failure, mid-stream included, fall back to software silently.
- **Color:** use the stream's own matrix, primaries, transfer and range. HDR passes through as scRGB `Rgba16Float` or is tone mapped to SDR.
- **Subtitles:** embedded tracks only, text and bitmap. Never load, add or edit subtitle files.
- **Export:**
  - Video packets are copied byte-for-byte. The one exception is the opt-in smart cut (H.264/HEVC): frames from the start up to the next keyframe are re-encoded (no B-frames, parameter sets in-band, the source's sent again before the first copied keyframe), and the editor states how many. Audio is copied unless volume or EQ changed; then only audio is re-encoded, and the export screen says so.
  - The UI shows where each cut actually lands before export. Never move a cut point silently.
  - Keep every stream (subtitles included), metadata and chapters unless the user drops them. Rebase timestamps to zero.
  - Write to a temp file, verify by probing, then rename. Never overwrite a source.
- **Single instance:** a second launch forwards its file to the running window over a per-user named pipe (Unix socket on Linux) and exits.
- **Updates:** check on a worker at every launch (and daily while the app stays open), never slowing startup; a newer release prompts right away. "Update" downloads the release's installer, runs it and exits; nothing downloads or installs otherwise. "Ask later" asks again on the next check. "Skip this version" stays quiet until a newer one. A failed check is silent.
- **Bad media:** files are untrusted. Corrupt input yields an error or a skipped frame, never a panic or hang.
- **FFmpeg:** raw pointers live in owning types with `Drop`; only safe APIs leave `media/src/ffi/`. Every `unsafe` block has a `// SAFETY:` comment.
- **Persisted files:** versioned; new fields use `#[serde(default)]`; breaking changes add a migration in `library`.
- **Errors:** one error enum per crate. No `unwrap`/`expect` outside tests unless the invariant is local and obvious.

## Code style

- `rustfmt` defaults, Clippy clean.
- Idiomatic Rust: iterators, `?`, enums over boolean flags, newtypes for units (`Ticks`, `FrameIndex`, `StreamIndex`, `SampleRate`), borrow over clone.
- Descriptive names over comments; comment only *why*. Doc comments only where behavior isn't obvious from the name and types.
- Files stay single-purpose and under 500 lines.

## Testing

- Test what can break in non-obvious ways: timebase math, keyframe snapping, cut planning and rebasing, seek generations, frame stepping, version comparison and update prompts, settings migrations, audio buffer sizing.
- Export tests check that copied packets are byte-identical to the source and the output probes as planned; smart-cut tests decode the output and compare every frame with the source.
- Don't test egui layout, trivial accessors, derived impls or third-party behavior. One test per behavior.
- Tests needing a GPU or real decoding are `#[ignore]` and use `tests/fixtures/`: each file under 1 MB, covering open GOP, B-frames, VFR, interlaced, multiple audio tracks, HDR, MP4/MKV/TS and a truncated file.

## Benchmarks

Ignored tests in `crates/app/src/smoke_tests/` that print a report and need no screen. Run with `cargo test --release -p app <name> -- --ignored --nocapture`.

- `playback_speed`, `seek_latency`, `frame_step_back`, `thumbnail_generation` — `PLAY_BENCH_FILES` (`;`-separated), `PLAY_BENCH_SOFTWARE_DECODE=1`
- `export_speed` — `PLAY_BENCH_FILES`, `PLAY_BENCH_AUDIO_REENCODE=1` (apply a volume change), `PLAY_BENCH_SMART_CUT=1`
- `random_seeks_never_panic`, `corrupt_files_never_panic`, `a_long_session_levels_off`; `random_seeks_never_panic_with_a_gpu` needs a GPU

## Working in this repo

- Grep/Glob first, then read only the relevant lines.
- Targeted edits; never rewrite a whole file for a small change. Don't reread a file you just edited.
- Keep changes small and on what was asked.

## Commits

[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/): `<type>(<scope>): <description>` — imperative, lowercase, no trailing period.

- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, `chore`, `style`
- Scope is the crate name: `feat(cut): snap trim start to previous keyframe`
- Breaking changes: `!` after the scope and a `BREAKING CHANGE:` footer
