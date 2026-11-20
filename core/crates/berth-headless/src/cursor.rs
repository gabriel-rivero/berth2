//! Cursor shape state from DECSCUSR (`CSI Ps SP q`).

/// The shapes DECSCUSR names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

/// The cursor shape and whether it blinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinks: bool,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            shape: CursorShape::Block,
            blinks: true,
        }
    }
}

impl CursorStyle {
    pub fn from_decscusr(param: u16) -> Self {
        match param {
            0 | 1 => Self {
                shape: CursorShape::Block,
                blinks: true,
            },
            2 => Self {
                shape: CursorShape::Block,
                blinks: false,
            },
            3 => Self {
                shape: CursorShape::Underline,
                blinks: true,
            },
            4 => Self {
                shape: CursorShape::Underline,
                blinks: false,
            },
            5 => Self {
                shape: CursorShape::Bar,
                blinks: true,
            },
            6 => Self {
                shape: CursorShape::Bar,
                blinks: false,
            },
            _ => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_each_decscusr_value_then_it_maps_to_shape_and_blink() {
        assert_eq!(
            CursorStyle::from_decscusr(1),
            CursorStyle {
                shape: CursorShape::Block,
                blinks: true
            }
        );
        assert_eq!(
            CursorStyle::from_decscusr(2),
            CursorStyle {
                shape: CursorShape::Block,
                blinks: false
            }
        );
        assert_eq!(
            CursorStyle::from_decscusr(3),
            CursorStyle {
                shape: CursorShape::Underline,
                blinks: true
            }
        );
        assert_eq!(
            CursorStyle::from_decscusr(4),
            CursorStyle {
                shape: CursorShape::Underline,
                blinks: false
            }
        );
        assert_eq!(
            CursorStyle::from_decscusr(5),
            CursorStyle {
                shape: CursorShape::Bar,
                blinks: true
            }
        );
        assert_eq!(
            CursorStyle::from_decscusr(6),
            CursorStyle {
                shape: CursorShape::Bar,
                blinks: false
            }
        );
    }
}
