/// How a video stream encodes color, as needed to convert it to display RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorInfo {
    pub matrix: Matrix,
    pub primaries: Primaries,
    pub transfer: Transfer,
    pub range: ColorRange,
}

impl ColorInfo {
    /// Fills unspecified fields the way most players do: HD and up is BT.709, SD is BT.601,
    /// and YUV without a range flag is limited range.
    pub fn resolved(self, height: u32) -> ColorInfo {
        let default_matrix = if height > 576 {
            Matrix::Bt709
        } else {
            Matrix::Bt601
        };
        let matrix = match self.matrix {
            Matrix::Unspecified => match self.primaries {
                Primaries::Bt2020 => Matrix::Bt2020,
                _ => default_matrix,
            },
            known => known,
        };
        let primaries = match self.primaries {
            Primaries::Unspecified => match matrix {
                Matrix::Bt2020 => Primaries::Bt2020,
                Matrix::Bt601 => Primaries::Bt601,
                _ => Primaries::Bt709,
            },
            known => known,
        };
        let range = match self.range {
            ColorRange::Unspecified if matrix == Matrix::Rgb => ColorRange::Full,
            ColorRange::Unspecified => ColorRange::Limited,
            known => known,
        };
        ColorInfo {
            matrix,
            primaries,
            transfer: self.transfer,
            range,
        }
    }

    pub fn is_hdr(&self) -> bool {
        matches!(self.transfer, Transfer::Pq | Transfer::Hlg)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
    Rgb,
    #[default]
    Unspecified,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Primaries {
    Bt601,
    Bt709,
    Bt2020,
    DciP3,
    #[default]
    Unspecified,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transfer {
    /// Any SDR gamma curve (BT.709, BT.601, sRGB, gamma 2.2); shown as-is.
    #[default]
    Sdr,
    Pq,
    Hlg,
    Linear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorRange {
    Limited,
    Full,
    #[default]
    Unspecified,
}

impl std::fmt::Display for ColorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let matrix = match self.matrix {
            Matrix::Bt601 => "BT.601",
            Matrix::Bt709 => "BT.709",
            Matrix::Bt2020 => "BT.2020",
            Matrix::Rgb => "RGB",
            Matrix::Unspecified => "unspecified",
        };
        let transfer = match self.transfer {
            Transfer::Sdr => "SDR",
            Transfer::Pq => "HDR10 (PQ)",
            Transfer::Hlg => "HLG",
            Transfer::Linear => "linear",
        };
        let range = match self.range {
            ColorRange::Limited => "limited",
            ColorRange::Full => "full",
            ColorRange::Unspecified => "unspecified",
        };
        write!(f, "{matrix}, {transfer}, {range} range")
    }
}
