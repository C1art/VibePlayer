# CLAUDE.md

## Project

A video and media player for Windows 11, built in Rust, with lossless cutting. Goal: playback on par with mpv/MPC-HC, and cut/trim/join without re-encoding like LosslessCut. No subtitle support.

## Stack

- Rust (stable, edition 2021), target `x86_64-pc-windows-msvc`
- UI: `egui` / `eframe`
- GPU: `wgpu` (DX12 primary, Vulkan fallback) + WGSL shaders
- Media: FFmpeg through our own safe wrapper over `ffmpeg-sys-next`, including `libavfilter`
- Audio: `cpal` for output, `rubato` for resampling
- Concurrency: `crossbeam` (channels, queues)
- Persistence: `serde` + `serde_json`
- Windows APIs: `windows` (D3D12/DXGI texture sharing, HDR output, SMTC, taskbar, file associations), as a `cfg(windows)` dependency
- Exe resources: `embed-resource` (build dependency of `app`) for the exe's icon and version info
- Ask before adding any dependency not listed here.

## Structure

Cargo workspace. Dependencies flow downward only; `cut` and `library` never depend on `app`, `render`, `media` or `playback`.

- `crates/app/` — eframe entry point, panels, OSD, input, shortcuts, OS integration
- `crates/media/` — FFmpeg wrapper: probe, demux, decode, filter, remux. All FFmpeg `unsafe` lives in `media/src/ffi/`
- `crates/playback/` — engine: clock, decode scheduling, seeking, frame stepping, speed
- `crates/render/` — wgpu presentation, color conversion, HDR; shaders in `render/shaders/*.wgsl`
- `crates/audio/` — cpal output, resampling, volume, EQ, downmix
- `crates/thumbnails/` — background seek-bar previews with a bounded cache
- `crates/cut/` — cut planning: segments, keyframe snapping, timestamp rebasing (pure logic, no I/O)
- `crates/export/` — runs a cut plan through `media`: stream copy, smart cut, join, verification
- `crates/library/` — settings, recents, resume positions, playlists, cut sessions in `%APPDATA%\Clplay`
- `tests/fixtures/` — small media files used by integration tests

## Commands

- Run: `cargo run -p app [-- <file>]`
- Release: `cargo build --release`
- Test: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all`
- FFmpeg: set `FFMPEG_DIR` to a shared FFmpeg build (8.1 or newer) before building

## Verification

After every change, in this order:

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test -p <changed crate>` (whole workspace before committing)

## Architecture rules

- **Time:** integer ticks with a rational timebase; never `f64` seconds outside display. Cut points use the source stream's timebase.
- **Threads:** the UI thread never decodes, does I/O or waits on a channel. Workers report back over `crossbeam` channels, polled each frame.
- **Audio callback:** no allocation, locks, blocking or logging. Reads a pre-filled lock-free queue only.
- **Clock:** audio is master; video drops or repeats frames to follow. No audio → monotonic system clock. Speed changes use `atempo`, never pitch-shifting.
- **Seeking:** seek to the prior keyframe, decode and discard to the target. Scrubbing uses keyframes only. A generation counter discards stale frames after each seek.
- **Hardware decode:** fall back to software on any failure, mid-stream included, silently.
- **Color:** use the stream's own matrix, primaries, transfer and range. HDR passes through as scRGB `Rgba16Float` or is tone mapped to SDR.
- **Lossless cut:** never re-encodes; packets are copied byte-for-byte. Smart cut is opt-in and states how many frames it re-encodes.
- **Keyframes:** the UI shows where a cut actually lands before export. Never move a cut point silently.
- **Export:** keep every stream, metadata and chapters unless dropped by the user; rebase timestamps to zero. Write to a temp file, verify by probing, then rename. Never overwrite a source.
- **Bad media:** files are untrusted. Corrupt input yields an error or a skipped frame, never a panic or hang.
- **FFmpeg:** raw pointers live in owning types with `Drop`; only safe APIs leave `media/src/ffi/`. Every `unsafe` block has a `// SAFETY:` comment.
- **Persisted files:** versioned; new fields use `#[serde(default)]`; breaking changes add a migration in `library`.
- **Errors:** one error enum per crate. No `unwrap`/`expect` outside tests unless the invariant is local and obvious.

## Code style

- Standard Rust conventions: `rustfmt` defaults, Clippy clean.
- Idiomatic Rust: iterators, `?`, enums over flag booleans, newtypes for units (`Ticks`, `FrameIndex`, `StreamIndex`, `SampleRate`), borrow over clone.
- Descriptive names over comments. Comment only *why*, never *what*.
- Doc comments only where behavior isn't obvious from the name and types.
- Files stay single-purpose and under 500 lines.

## Testing

- Test what can break in non-obvious ways: timebase math, keyframe snapping, cut planning and rebasing, seek generations, frame stepping, playlist parsing, settings migrations, audio buffer sizing.
- Lossless export tests check copied packets are byte-identical to the source and the output probes as planned.
- Don't test egui layout, trivial accessors, derived impls or third-party behavior. One test per behavior.
- Tests needing a GPU or real decoding are `#[ignore]` and use `tests/fixtures/` (each file under 1 MB; cover open GOP, B-frames, VFR, interlaced, multiple audio tracks, HDR, MP4/MKV/TS, a truncated file).

## Working in this repo

- Search before reading: Grep/Glob to locate code, then read only the relevant lines.
- Edit in place with targeted edits. Never rewrite a whole file for a small change.
- Don't reread a file you just edited.
- Keep changes small and on what was asked.

## Benchmarks

Ignored tests in `crates/app/src/smoke_tests/` that print a report and need no screen. Run with `cargo test --release -p app <n> -- --ignored --nocapture`.

- `playback_speed`, `seek_latency`, `frame_step_back`, `thumbnail_generation` — `PLAY_BENCH_FILES` (`;`-separated), `PLAY_BENCH_SOFTWARE_DECODE=1`
- `export_speed` — `PLAY_BENCH_FILES`, `PLAY_BENCH_SEGMENTS` (default 10), `PLAY_BENCH_SMART_CUT=1`
- `random_seeks_never_panic`, `corrupt_files_never_panic`, `a_long_session_levels_off`; `_with_a_gpu` variants need a GPU

## Commits

[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/): `<type>(<scope>): <description>`, imperative, lowercase, no trailing period.

- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, `chore`, `style`
- Scope is the crate name: `feat(cut): snap segment start to previous keyframe`
- Breaking changes: `!` after the scope and a `BREAKING CHANGE:` footer

## Don't

- Block the UI thread or audio callback — use a worker.
- Let FFmpeg pointers or `unsafe` escape `media/src/ffi/` — add a safe wrapper.
- Use floating-point time in the model — use `Ticks`.
- Re-encode in a lossless cut or move a cut point without showing it.
- Overwrite a source file — temp file, verify, rename.
- Panic on bad media — return an error or skip the frame.
