use serde::{Deserialize, Serialize};

/// Player volume as a percentage of full scale, 0–100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Volume(u8);

impl Volume {
    pub const MAX: Volume = Volume(100);

    pub fn from_percent(percent: i32) -> Volume {
        Volume(percent.clamp(0, 100) as u8)
    }

    pub fn percent(self) -> u8 {
        self.0
    }

    pub fn step(self, delta: i32) -> Volume {
        Volume::from_percent(i32::from(self.0) + delta)
    }

    /// Linear amplitude. Squaring makes equal slider steps sound roughly equally loud.
    pub fn gain(self) -> f32 {
        let fraction = f32::from(self.0) / 100.0;
        fraction * fraction
    }
}

impl Default for Volume {
    fn default() -> Volume {
        Volume(80)
    }
}

#[cfg(test)]
mod tests {
    use super::Volume;

    #[test]
    fn volume_is_clamped_and_gain_is_perceptual() {
        assert_eq!(Volume::from_percent(150), Volume::MAX);
        assert_eq!(Volume::from_percent(-5).gain(), 0.0);
        assert_eq!(Volume::from_percent(50).gain(), 0.25);
        assert_eq!(Volume::MAX.gain(), 1.0);
    }
}
