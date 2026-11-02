use cut::{ClockPrecision, ClockTime, Timebase};
use eframe::egui::{self, RichText};
use media::{MediaInfo, StreamDetails, StreamInfo, StreamKind};

use crate::theme;

/// Live numbers shown next to the file's static description.
pub struct PlaybackStats {
    pub hardware_decoding: Option<bool>,
    pub dropped_frames: u64,
    pub audio_error: Option<String>,
}

pub fn show(ctx: &egui::Context, open: &mut bool, info: &MediaInfo, stats: &PlaybackStats) {
    crate::widgets::window("File info")
        .open(open)
        .resizable(true)
        .default_width(380.0)
        .default_height(ctx.content_rect().height() * 0.7)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-400.0, 20.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| contents(ui, info, stats));
        });
}

fn contents(ui: &mut egui::Ui, info: &MediaInfo, stats: &PlaybackStats) {
    section(ui, "General", |rows| {
        rows.row("File", info.file_name());
        if let Some(size) = info.file_size {
            rows.row("Size", format_bytes(size));
        }
        rows.row("Container", info.format_long.clone());
        if let Some(duration) = info.duration {
            rows.row(
                "Duration",
                ClockTime::new(duration, Timebase::MICROS, ClockPrecision::Millis).to_string(),
            );
        }
        if let Some(bit_rate) = info.bit_rate {
            rows.row("Overall bitrate", format_bit_rate(bit_rate));
        }
        if !info.chapters.is_empty() {
            rows.row("Chapters", info.chapters.len().to_string());
        }
    });
    for stream in &info.streams {
        let title = match stream.kind {
            StreamKind::Video => "Video",
            StreamKind::Audio => "Audio",
            StreamKind::Subtitle => "Subtitles",
            StreamKind::Other => continue,
        };
        let heading = format!("{title} #{}", stream.index.0);
        section(ui, &heading, |rows| stream_rows(rows, stream, stats));
    }
    if !info.metadata.is_empty() {
        section(ui, "Metadata", |rows| {
            for (key, value) in &info.metadata {
                rows.row(key, value.clone());
            }
        });
    }
}

fn stream_rows(rows: &mut Rows<'_>, stream: &StreamInfo, stats: &PlaybackStats) {
    // Long names list every alias ("H.264 / AVC / MPEG-4 AVC / …"); the first is enough.
    let name = stream
        .codec_long
        .as_deref()
        .and_then(|long| long.split(" / ").next())
        .unwrap_or(&stream.codec);
    let codec = match &stream.profile {
        Some(profile) => format!("{name} ({profile})"),
        None => name.to_owned(),
    };
    rows.row("Codec", codec);
    match &stream.details {
        StreamDetails::Video(video) => {
            rows.row("Resolution", format!("{} × {}", video.width, video.height));
            if let Some((num, den)) = video.frame_rate {
                let fps = f64::from(num) / f64::from(den);
                let variable = if video.variable_frame_rate {
                    " (variable)"
                } else {
                    ""
                };
                rows.row("Frame rate", format!("{fps:.3} fps{variable}"));
            }
            if let Some(format) = &video.pixel_format {
                let depth = video
                    .bit_depth
                    .map(|bits| format!(", {bits}-bit"))
                    .unwrap_or_default();
                rows.row("Pixel format", format!("{format}{depth}"));
            }
            rows.row("Color", video.color.resolved(video.height).to_string());
            if video.interlaced {
                rows.row("Scan", "Interlaced (deinterlaced for display)".into());
            }
            if let Some((num, den)) = video.sample_aspect_ratio
                && num != den
            {
                rows.row("Pixel aspect", format!("{num}:{den}"));
            }
            if !video.attached_picture {
                if let Some(hardware) = stats.hardware_decoding {
                    let decoder = if hardware { "Hardware" } else { "Software" };
                    rows.row("Decoding", decoder.into());
                }
                rows.row("Dropped frames", stats.dropped_frames.to_string());
            } else {
                rows.row("Kind", "Cover art".into());
            }
        }
        StreamDetails::Audio(audio) => {
            rows.row("Sample rate", format!("{} Hz", audio.sample_rate));
            let layout = if audio.layout.is_empty() {
                channels(audio.channels)
            } else {
                format!("{} ({})", audio.layout, channels(audio.channels))
            };
            rows.row("Channels", layout);
            if let Some(format) = &audio.sample_format {
                rows.row("Sample format", format.clone());
            }
            if let Some(error) = &stats.audio_error {
                rows.row("Output", error.clone());
            }
        }
        StreamDetails::Subtitle { bitmap, .. } => {
            let kind = if *bitmap { "Image" } else { "Text" };
            rows.row("Type", kind.into());
        }
        StreamDetails::Other => {}
    }
    if let Some(bit_rate) = stream.bit_rate {
        rows.row("Bitrate", format_bit_rate(bit_rate));
    }
    if let Some(language) = &stream.language {
        rows.row("Language", language.clone());
    }
    if let Some(title) = &stream.title {
        rows.row("Title", title.clone());
    }
    if stream.is_default {
        rows.row("Default", "Yes".into());
    }
}

struct Rows<'a> {
    ui: &'a mut egui::Ui,
}

impl Rows<'_> {
    fn row(&mut self, label: &str, value: String) {
        self.ui.label(RichText::new(label).color(theme::TEXT_DIM));
        self.ui.add(egui::Label::new(value).wrap());
        self.ui.end_row();
    }
}

fn section(ui: &mut egui::Ui, title: &str, add_rows: impl FnOnce(&mut Rows<'_>)) {
    ui.add_space(4.0);
    crate::widgets::section_title(ui, title);
    egui::Grid::new(title)
        .num_columns(2)
        .spacing([16.0, 4.0])
        .min_col_width(110.0)
        .show(ui, |ui| add_rows(&mut Rows { ui }));
    ui.add_space(6.0);
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn format_bit_rate(bits_per_second: u64) -> String {
    if bits_per_second >= 1_000_000 {
        format!("{:.2} Mb/s", bits_per_second as f64 / 1_000_000.0)
    } else {
        format!("{} kb/s", bits_per_second / 1000)
    }
}

fn channels(count: u32) -> String {
    if count == 1 {
        "1 channel".into()
    } else {
        format!("{count} channels")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_bitrates_use_readable_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1_572_864), "1.50 MB");
        assert_eq!(format_bit_rate(128_000), "128 kb/s");
        assert_eq!(format_bit_rate(8_500_000), "8.50 Mb/s");
    }
}
