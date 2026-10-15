//! Logical-pixel sizing for the pet and its attached work note.
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    pub pet_side: f32,
}
pub fn geometry(scale: u16, show: bool, compact: bool) -> Geometry {
    let pet_side = if show { 160.0 } else { 180.0 } * f32::from(scale.clamp(50, 200)) / 100.0;
    if show {
        Geometry {
            width: pet_side.max(320.0),
            height: pet_side + if compact { 116.0 } else { 276.0 },
            pet_side,
        }
    } else {
        Geometry {
            width: pet_side,
            height: pet_side + 28.0,
            pet_side,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn note_stays_readable_at_small_pet_scales_and_compacts_without_disappearing() {
        let expanded = geometry(50, true, false);
        let compact = geometry(50, true, true);
        assert_eq!(expanded.width, 320.0);
        assert!(expanded.height > compact.height);
        assert!(compact.height > compact.pet_side);
        assert_eq!(geometry(100, false, false).width, 180.0);
        assert!(geometry(200, true, false).height < 650.0);
    }
}
