//! Portable per-view magnification policy shared by menus and frontends.
use super::LayoutError;

pub const ZOOM_STOPS: [f32; 17] = [
    0.25, 0.33, 0.50, 0.67, 0.75, 0.80, 0.90, 1.00, 1.10, 1.25, 1.50, 1.75, 2.00, 2.50, 3.00, 4.00,
    5.00,
];

pub fn valid_zoom_scale(scale: f32) -> bool {
    scale.is_finite() && (ZOOM_STOPS[0]..=ZOOM_STOPS[ZOOM_STOPS.len() - 1]).contains(&scale)
}

/// Choose the next strictly adjacent stop, saturating at either endpoint.
/// Arbitrary valid scales remain supported by the direct scale setter.
pub fn adjacent_zoom_scale(scale: f32, increasing: bool) -> Result<f32, LayoutError> {
    if !valid_zoom_scale(scale) {
        return Err(LayoutError::InvalidScale);
    }
    Ok(if increasing {
        ZOOM_STOPS
            .into_iter()
            .find(|stop| *stop > scale)
            .unwrap_or(ZOOM_STOPS[16])
    } else {
        ZOOM_STOPS
            .into_iter()
            .rev()
            .find(|stop| *stop < scale)
            .unwrap_or(ZOOM_STOPS[0])
    })
}
