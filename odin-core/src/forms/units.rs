//! Unit conversions between page measurement units and CSS pixels.

const DPI: f64 = 96.0;

/// Pixels-per-unit conversion factor for a supported unit.
fn factor(unit: &str) -> Option<f64> {
    match unit {
        "inch" => Some(DPI),
        "cm" => Some(DPI / 2.54),
        "mm" => Some(DPI / 25.4),
        "pt" => Some(DPI / 72.0),
        _ => None,
    }
}

/// Round to three decimal places.
fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Convert a value in the given page unit to CSS pixels.
///
/// # Panics
///
/// Panics if `unit` is not one of `inch`, `cm`, `mm`, `pt`.
pub fn to_pixels(value: f64, unit: &str) -> f64 {
    let f = factor(unit).unwrap_or_else(|| panic!("Unknown unit: {unit}"));
    round3(value * f)
}

/// Convert CSS pixels back to the given page unit.
///
/// # Panics
///
/// Panics if `unit` is not one of `inch`, `cm`, `mm`, `pt`.
pub fn from_pixels(px: f64, unit: &str) -> f64 {
    let f = factor(unit).unwrap_or_else(|| panic!("Unknown unit: {unit}"));
    round3(px / f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inch_to_pixels() {
        assert_eq!(to_pixels(1.0, "inch"), 96.0);
        assert_eq!(to_pixels(8.5, "inch"), 816.0);
    }

    #[test]
    fn point_to_pixels() {
        assert_eq!(to_pixels(72.0, "pt"), 96.0);
        assert_eq!(to_pixels(12.0, "pt"), 16.0);
    }

    #[test]
    fn metric_to_pixels() {
        assert_eq!(to_pixels(2.54, "cm"), 96.0);
        assert_eq!(to_pixels(25.4, "mm"), 96.0);
    }

    #[test]
    fn round_trip() {
        assert_eq!(from_pixels(96.0, "inch"), 1.0);
        assert_eq!(from_pixels(16.0, "pt"), 12.0);
    }

    #[test]
    #[should_panic(expected = "Unknown unit")]
    fn unknown_unit_panics() {
        to_pixels(1.0, "furlong");
    }
}
