//! Small white transport icons drawn in code, for the taskbar's thumbnail buttons.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Play,
    Pause,
    Back,
    Forward,
}

type Point = (f32, f32);

impl Glyph {
    /// Convex shapes in a unit square, y down.
    fn shapes(self) -> Vec<Vec<Point>> {
        let bar =
            |left: f32, right: f32| vec![(left, 0.2), (right, 0.2), (right, 0.8), (left, 0.8)];
        let right = |left: f32, tip: f32| vec![(left, 0.25), (tip, 0.5), (left, 0.75)];
        let left = |right: f32, tip: f32| vec![(right, 0.25), (right, 0.75), (tip, 0.5)];
        match self {
            Glyph::Play => vec![vec![(0.28, 0.18), (0.82, 0.5), (0.28, 0.82)]],
            Glyph::Pause => vec![bar(0.25, 0.42), bar(0.58, 0.75)],
            Glyph::Forward => vec![right(0.12, 0.5), right(0.5, 0.88)],
            Glyph::Back => vec![left(0.5, 0.12), left(0.88, 0.5)],
        }
    }
}

/// `glyph` in white on transparent, `size` × `size` pixels of BGRA with straight alpha, rows
/// from the top, edges smoothed by sampling each pixel 4 × 4 times.
pub fn render(glyph: Glyph, size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let shapes = glyph.shapes();
    let size = size.max(1);
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut covered = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let point = (
                        (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32,
                        (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32,
                    );
                    if shapes.iter().any(|shape| contains(shape, point)) {
                        covered += 1;
                    }
                }
            }
            let alpha = (covered * 255 / (SAMPLES * SAMPLES)) as u8;
            pixels.extend_from_slice(&[255, 255, 255, alpha]);
        }
    }
    pixels
}

/// Whether a convex polygon, wound either way, contains `point`.
fn contains(shape: &[Point], point: Point) -> bool {
    let sides = shape
        .iter()
        .zip(shape.iter().cycle().skip(1))
        .map(|(a, b)| (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0));
    let mut sign = 0.0f32;
    for side in sides {
        if side != 0.0 {
            if sign != 0.0 && side.signum() != sign {
                return false;
            }
            sign = side.signum();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(pixels: &[u8], size: u32, x: u32, y: u32) -> u8 {
        pixels[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn glyphs_cover_their_shapes_and_leave_the_rest_clear() {
        let play = render(Glyph::Play, 16);
        assert_eq!(play.len(), 16 * 16 * 4);
        assert_eq!(alpha(&play, 16, 7, 8), 255);
        assert_eq!(alpha(&play, 16, 0, 0), 0);
        assert_eq!(alpha(&play, 16, 14, 2), 0);
        let pause = render(Glyph::Pause, 16);
        assert_eq!(alpha(&pause, 16, 5, 8), 255);
        assert_eq!(alpha(&pause, 16, 8, 8), 0);
    }
}
