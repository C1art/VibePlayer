use cut::{Ticks, Timebase};

use crate::MediaError;
use crate::ffi::{CodecContext, Packet, RawSubtitleRect, StreamParams};

/// A subtitle event on the media timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Subtitle {
    /// In microseconds.
    pub start: Ticks,
    /// In microseconds; `None` means until the next event.
    pub end: Option<Ticks>,
    pub content: SubtitleContent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubtitleContent {
    Text(String),
    /// Images positioned on a `canvas_width` × `canvas_height` canvas that maps onto the video.
    Bitmaps {
        canvas_width: u32,
        canvas_height: u32,
        images: Vec<SubtitleBitmap>,
    },
    /// Removes whatever is showing.
    Clear,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitleBitmap {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA.
    pub rgba: Vec<u8>,
}

/// Decodes an embedded subtitle stream.
pub struct SubtitleDecoder {
    codec: CodecContext,
    timebase: Timebase,
    canvas: (u32, u32),
}

impl SubtitleDecoder {
    /// `video_size` is used as the bitmap canvas when the stream doesn't state its own.
    pub fn new(
        params: StreamParams,
        video_size: (u32, u32),
    ) -> Result<SubtitleDecoder, MediaError> {
        let view = params.view();
        let canvas = match (view.width(), view.height()) {
            (0, _) | (_, 0) => video_size,
            size => size,
        };
        Ok(SubtitleDecoder {
            codec: CodecContext::decoder(&params, None)?,
            timebase: params.timebase(),
            canvas,
        })
    }

    /// Decodes one packet. Undecodable packets yield `Ok(None)`.
    pub fn decode(&mut self, packet: &Packet) -> Result<Option<Subtitle>, MediaError> {
        let Some(pts) = packet.pts() else {
            return Ok(None);
        };
        let Ok(Some(raw)) = self.codec.decode_subtitle(packet) else {
            return Ok(None);
        };
        let base = pts.rescale(self.timebase, Timebase::MICROS);
        let start = base + millis(raw.start_millis);
        let end = raw.end_millis.map(|end| base + millis(end)).or_else(|| {
            (packet.duration().0 > 0)
                .then(|| base + packet.duration().rescale(self.timebase, Timebase::MICROS))
        });
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for rect in raw.rects {
            match rect {
                RawSubtitleRect::Text(line) => texts.push(ass_to_plain_text(&line)),
                RawSubtitleRect::Bitmap {
                    x,
                    y,
                    width,
                    height,
                    rgba,
                } => images.push(SubtitleBitmap {
                    x,
                    y,
                    width,
                    height,
                    rgba,
                }),
            }
        }
        let content = if !images.is_empty() {
            SubtitleContent::Bitmaps {
                canvas_width: self.canvas.0,
                canvas_height: self.canvas.1,
                images,
            }
        } else if texts.iter().any(|t| !t.is_empty()) {
            SubtitleContent::Text(texts.join("\n"))
        } else {
            SubtitleContent::Clear
        };
        Ok(Some(Subtitle {
            start,
            end,
            content,
        }))
    }

    pub fn flush(&mut self) {
        self.codec.flush();
    }
}

fn millis(value: u32) -> Ticks {
    Ticks(i64::from(value) * 1000)
}

/// Turns an ASS dialogue line into plain text: drops the leading fields and `{...}` override
/// tags, and converts `\N`, `\n` and `\h`.
fn ass_to_plain_text(line: &str) -> String {
    const LEADING_FIELDS: usize = 8;
    let text = if line.starts_with("Dialogue:") || line.matches(',').count() >= LEADING_FIELDS {
        line.splitn(LEADING_FIELDS + 1, ',')
            .nth(LEADING_FIELDS)
            .unwrap_or(line)
    } else {
        line
    };
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => in_tag = true,
            '}' if in_tag => in_tag = false,
            _ if in_tag => {}
            '\\' => match chars.peek() {
                Some('N' | 'n') => {
                    chars.next();
                    plain.push('\n');
                }
                Some('h') => {
                    chars.next();
                    plain.push(' ');
                }
                _ => plain.push('\\'),
            },
            _ => plain.push(c),
        }
    }
    plain.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::ass_to_plain_text;

    #[test]
    fn ass_dialogue_fields_and_override_tags_are_stripped() {
        let line = r"0,0,Default,,0,0,0,,{\i1}Hello{\i0}, world\Nsecond\hline";
        assert_eq!(ass_to_plain_text(line), "Hello, world\nsecond line");
    }

    #[test]
    fn plain_text_without_fields_is_kept() {
        assert_eq!(ass_to_plain_text("just text"), "just text");
    }
}
