use media::{ColorInfo, ColorRange, Matrix, PlaneLayout, Primaries, Transfer};

/// Brightness of SDR white in nits, per ITU-R BT.2408.
const SDR_WHITE_NITS: f32 = 203.0;
const DEFAULT_HDR_PEAK_NITS: f32 = 1000.0;

/// The shader's uniform block, laid out as 4-float rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorParams {
    pub rows: [[f32; 4]; 3],
    pub range: [f32; 4],
    pub gamut: [[f32; 4]; 3],
    pub misc: [f32; 4],
    pub flags: [f32; 4],
}

impl ColorParams {
    pub fn new(
        color: ColorInfo,
        layout: PlaneLayout,
        peak_nits: Option<f32>,
        target_is_srgb: bool,
    ) -> ColorParams {
        let (kr, kb) = match color.matrix {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt2020 => (0.2627, 0.0593),
            Matrix::Bt709 | Matrix::Rgb | Matrix::Unspecified => (0.2126, 0.0722),
        };
        let kg = 1.0 - kr - kb;
        let rows = [
            [1.0, 0.0, 2.0 * (1.0 - kr), 0.0],
            [
                1.0,
                -2.0 * kb * (1.0 - kb) / kg,
                -2.0 * kr * (1.0 - kr) / kg,
                0.0,
            ],
            [1.0, 2.0 * (1.0 - kb), 0.0, 0.0],
        ];
        let max_code = ((1u32 << layout.bits) - 1) as f32;
        let unit = (1u32 << (layout.bits - 8)) as f32 / max_code;
        let range = match color.range {
            ColorRange::Full => [0.0, 1.0, 128.0 * unit, 1.0],
            ColorRange::Limited | ColorRange::Unspecified => [
                16.0 * unit,
                1.0 / (219.0 * unit),
                128.0 * unit,
                1.0 / (224.0 * unit),
            ],
        };
        let sample_scale = if layout.bits > 8 {
            65535.0 / max_code
        } else {
            1.0
        };
        let transfer = match color.transfer {
            Transfer::Pq => 1.0,
            Transfer::Hlg => 2.0,
            Transfer::Sdr | Transfer::Linear => 0.0,
        };
        let gamut = gamut_to_bt709(color.primaries);
        let wide_gamut = gamut != IDENTITY;
        let peak = peak_nits.unwrap_or(DEFAULT_HDR_PEAK_NITS) / SDR_WHITE_NITS;
        ColorParams {
            rows,
            range,
            gamut,
            misc: [
                sample_scale,
                transfer,
                peak.max(1.0),
                if target_is_srgb { 1.0 } else { 0.0 },
            ],
            flags: [
                if color.is_hdr() || wide_gamut {
                    1.0
                } else {
                    0.0
                },
                if layout.interleaved_chroma { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
        }
    }

    pub fn to_bytes(self) -> Vec<u8> {
        self.rows
            .iter()
            .chain([&self.range])
            .chain(self.gamut.iter())
            .chain([&self.misc, &self.flags])
            .flatten()
            .flat_map(|value| value.to_ne_bytes())
            .collect()
    }
}

const IDENTITY: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// Linear RGB conversion from the source primaries to BT.709/sRGB primaries.
fn gamut_to_bt709(primaries: Primaries) -> [[f32; 4]; 3] {
    match primaries {
        Primaries::Bt2020 => [
            [1.6605, -0.5876, -0.0728, 0.0],
            [-0.1246, 1.1329, -0.0083, 0.0],
            [-0.0182, -0.1006, 1.1187, 0.0],
        ],
        Primaries::DciP3 => [
            [1.2249, -0.2247, 0.0, 0.0],
            [-0.0420, 1.0419, 0.0, 0.0],
            [-0.0197, -0.0786, 1.0979, 0.0],
        ],
        Primaries::Bt601 | Primaries::Bt709 | Primaries::Unspecified => IDENTITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EIGHT_BIT: PlaneLayout = PlaneLayout {
        bits: 8,
        chroma_shift_x: 1,
        chroma_shift_y: 1,
        interleaved_chroma: false,
    };

    fn bt709_limited() -> ColorInfo {
        ColorInfo {
            matrix: Matrix::Bt709,
            primaries: Primaries::Bt709,
            transfer: Transfer::Sdr,
            range: ColorRange::Limited,
        }
    }

    /// Mirrors the shader's SDR path for one 8-bit sample.
    fn convert(params: &ColorParams, y: f32, u: f32, v: f32) -> [f32; 3] {
        let normalize = |code: f32| code / 255.0 * params.misc[0];
        let luma = (normalize(y) - params.range[0]) * params.range[1];
        let cb = (normalize(u) - params.range[2]) * params.range[3];
        let cr = (normalize(v) - params.range[2]) * params.range[3];
        params
            .rows
            .map(|row| (row[0] * luma + row[1] * cb + row[2] * cr).clamp(0.0, 1.0))
    }

    fn assert_close(actual: [f32; 3], expected: [f32; 3]) {
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 0.01, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn limited_range_black_and_white_map_to_the_ends() {
        let params = ColorParams::new(bt709_limited(), EIGHT_BIT, None, false);
        assert_close(convert(&params, 16.0, 128.0, 128.0), [0.0, 0.0, 0.0]);
        assert_close(convert(&params, 235.0, 128.0, 128.0), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn bt709_red_converts_to_red() {
        let params = ColorParams::new(bt709_limited(), EIGHT_BIT, None, false);
        // BT.709 limited-range codes for pure red.
        assert_close(convert(&params, 63.0, 102.0, 240.0), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn full_range_uses_all_codes() {
        let full = ColorInfo {
            range: ColorRange::Full,
            ..bt709_limited()
        };
        let params = ColorParams::new(full, EIGHT_BIT, None, false);
        assert_close(convert(&params, 255.0, 128.0, 128.0), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn ten_bit_samples_are_rescaled_from_sixteen_bit_textures() {
        let ten_bit = PlaneLayout {
            bits: 10,
            ..EIGHT_BIT
        };
        let params = ColorParams::new(bt709_limited(), ten_bit, None, false);
        // A 10-bit code of 940 (limited white) read from an R16Unorm texture.
        let sampled = 940.0 / 65535.0 * params.misc[0];
        let luma = (sampled - params.range[0]) * params.range[1];
        assert!((luma - 1.0).abs() < 0.001, "{luma}");
    }

    #[test]
    fn only_hdr_or_wide_gamut_takes_the_linear_path() {
        let sdr = ColorParams::new(bt709_limited(), EIGHT_BIT, None, false);
        assert_eq!(sdr.flags[0], 0.0);
        let hdr = ColorInfo {
            transfer: Transfer::Pq,
            primaries: Primaries::Bt2020,
            ..bt709_limited()
        };
        let hdr = ColorParams::new(hdr, EIGHT_BIT, Some(4000.0), false);
        assert_eq!(hdr.flags[0], 1.0);
        assert!((hdr.misc[2] - 4000.0 / 203.0).abs() < 0.01);
    }

    #[test]
    fn interleaved_chroma_is_flagged_for_the_shader() {
        let nv12 = PlaneLayout {
            interleaved_chroma: true,
            ..EIGHT_BIT
        };
        assert_eq!(
            ColorParams::new(bt709_limited(), nv12, None, false).flags[1],
            1.0
        );
        assert_eq!(
            ColorParams::new(bt709_limited(), EIGHT_BIT, None, false).flags[1],
            0.0
        );
    }

    #[test]
    fn uniform_block_is_nine_rows_of_four_floats() {
        let params = ColorParams::new(bt709_limited(), EIGHT_BIT, None, false);
        assert_eq!(params.to_bytes().len(), 9 * 16);
    }
}
