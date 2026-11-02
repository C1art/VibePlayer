use ffmpeg_sys_next as sys;

use crate::color::{ColorInfo, ColorRange, Matrix, Primaries, Transfer};

pub(super) fn color_info(
    space: sys::AVColorSpace,
    primaries: sys::AVColorPrimaries,
    transfer: sys::AVColorTransferCharacteristic,
    range: sys::AVColorRange,
) -> ColorInfo {
    use sys::AVColorPrimaries as P;
    use sys::AVColorSpace as S;
    use sys::AVColorTransferCharacteristic as T;
    ColorInfo {
        matrix: match space {
            S::AVCOL_SPC_BT709 | S::AVCOL_SPC_SMPTE240M => Matrix::Bt709,
            S::AVCOL_SPC_BT470BG | S::AVCOL_SPC_SMPTE170M | S::AVCOL_SPC_FCC => Matrix::Bt601,
            S::AVCOL_SPC_BT2020_NCL | S::AVCOL_SPC_BT2020_CL => Matrix::Bt2020,
            S::AVCOL_SPC_RGB => Matrix::Rgb,
            _ => Matrix::Unspecified,
        },
        primaries: match primaries {
            P::AVCOL_PRI_BT709 => Primaries::Bt709,
            P::AVCOL_PRI_BT470BG | P::AVCOL_PRI_SMPTE170M | P::AVCOL_PRI_SMPTE240M => {
                Primaries::Bt601
            }
            P::AVCOL_PRI_BT2020 => Primaries::Bt2020,
            P::AVCOL_PRI_SMPTE431 | P::AVCOL_PRI_SMPTE432 => Primaries::DciP3,
            _ => Primaries::Unspecified,
        },
        transfer: match transfer {
            T::AVCOL_TRC_SMPTE2084 => Transfer::Pq,
            T::AVCOL_TRC_ARIB_STD_B67 => Transfer::Hlg,
            T::AVCOL_TRC_LINEAR => Transfer::Linear,
            _ => Transfer::Sdr,
        },
        range: match range {
            sys::AVColorRange::AVCOL_RANGE_MPEG => ColorRange::Limited,
            sys::AVColorRange::AVCOL_RANGE_JPEG => ColorRange::Full,
            _ => ColorRange::Unspecified,
        },
    }
}
