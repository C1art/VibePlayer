//! How a video is turned for display, from the container's display matrix (phone videos are
//! usually stored sideways with one).

/// A clockwise turn in quarters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    None,
    Clockwise90,
    Half,
    Clockwise270,
}

impl Rotation {
    /// From the counterclockwise angle FFmpeg reads from a display matrix, rounded to the
    /// nearest quarter turn.
    pub fn from_counterclockwise_degrees(degrees: f64) -> Rotation {
        if !degrees.is_finite() {
            return Rotation::None;
        }
        let quarters = (-degrees / 90.0).round().rem_euclid(4.0);
        match quarters as u8 {
            1 => Rotation::Clockwise90,
            2 => Rotation::Half,
            3 => Rotation::Clockwise270,
            _ => Rotation::None,
        }
    }

    pub fn quarter_turns(self) -> u8 {
        match self {
            Rotation::None => 0,
            Rotation::Clockwise90 => 1,
            Rotation::Half => 2,
            Rotation::Clockwise270 => 3,
        }
    }

    /// Whether width and height trade places on screen.
    pub fn is_sideways(self) -> bool {
        self.quarter_turns() % 2 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::Rotation;

    #[test]
    fn display_matrix_angles_map_to_clockwise_quarter_turns() {
        // A portrait phone video: the matrix turns it 90° clockwise, which FFmpeg reads as -90.
        assert_eq!(
            Rotation::from_counterclockwise_degrees(-90.0),
            Rotation::Clockwise90
        );
        assert_eq!(
            Rotation::from_counterclockwise_degrees(90.0),
            Rotation::Clockwise270
        );
        assert_eq!(
            Rotation::from_counterclockwise_degrees(180.0),
            Rotation::Half
        );
        assert_eq!(
            Rotation::from_counterclockwise_degrees(-180.0),
            Rotation::Half
        );
        assert_eq!(
            Rotation::from_counterclockwise_degrees(-0.0),
            Rotation::None
        );
        assert_eq!(
            Rotation::from_counterclockwise_degrees(f64::NAN),
            Rotation::None
        );
    }
}
