//! What goes into the log file: enough to tell what happened on someone else's machine.

use library::log;
use media::{MediaInfo, StreamDetails};
use playback::Player;

/// The app version, system, GPU and FFmpeg build.
pub fn log_startup(creation: &eframe::CreationContext<'_>, high_bit_depth: Option<bool>) {
    log::info(format!(
        "VibePlayer {} on {} {}, FFmpeg {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        media::ffmpeg_version()
    ));
    match &creation.wgpu_render_state {
        Some(state) => {
            let adapter = state.adapter.get_info();
            log::info(format!(
                "GPU {} ({:?}, driver {}), output {:?}, 16-bit video textures: {}, \
                 decoded frames can stay on the GPU: {}",
                adapter.name,
                adapter.backend,
                adapter.driver_info,
                state.target_format,
                high_bit_depth.unwrap_or(false),
                render::gpu_sharing(&state.device).is_some()
            ));
        }
        None => log::warn("no GPU renderer"),
    }
}

/// Logs panics before the default hook prints them (to a console a release build doesn't have).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::write_now(info);
        default(info);
    }));
}

/// One line on a file that opened: container, duration and the streams' codecs.
pub fn describe(info: &MediaInfo) -> String {
    let streams: Vec<String> = info
        .streams
        .iter()
        .map(|stream| match &stream.details {
            StreamDetails::Video(video) => format!(
                "video {} {}x{}{}",
                stream.codec,
                video.width,
                video.height,
                video
                    .pixel_format
                    .as_deref()
                    .map(|format| format!(" {format}"))
                    .unwrap_or_default()
            ),
            StreamDetails::Audio(audio) => {
                format!(
                    "audio {} {} Hz {} ch",
                    stream.codec, audio.sample_rate, audio.channels
                )
            }
            _ => format!("{:?} {}", stream.kind, stream.codec).to_lowercase(),
        })
        .collect();
    let seconds = info.duration.map_or(0.0, |d| d.0 as f64 / 1e6);
    format!(
        "{}: {}, {seconds:.1} s, {}",
        info.path.display(),
        info.format,
        streams.join(", ")
    )
}

/// Notes changes in how a file is playing, once each.
#[derive(Default)]
pub struct PlaybackLog {
    hardware: Option<bool>,
    audio_error: bool,
}

impl PlaybackLog {
    pub fn observe(&mut self, player: &Player) {
        if let Some(hardware) = player.hardware_decoding()
            && self.hardware != Some(hardware)
        {
            if self.hardware.is_some() {
                log::warn("hardware decoding failed; switched to software");
            } else {
                log::info(format!(
                    "{} decoding",
                    if hardware { "hardware" } else { "software" }
                ));
            }
            self.hardware = Some(hardware);
        }
        if let Some(error) = player.audio_error()
            && !self.audio_error
        {
            self.audio_error = true;
            log::warn(format!("audio unavailable: {error}"));
        }
    }
}
