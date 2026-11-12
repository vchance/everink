//! The camera: an exact center, and a zoom held in a fixed range by changing level.
//!
//! The camera is local state. Its zoom is a float and may drift; its center is exact. `normalize` keeps the zoom in
//! `[0.5, 2048)` pixels per unit. The range is wider than one level (1024) on purpose: after a level change the
//! zoom must move by a factor of four before the level changes back, so a camera resting at a boundary does not
//! change level on every frame.

use crate::{Axis, Delta, LEVEL_SHIFT, Level, Position, Rel, UNIT_BITS};

pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2048.0;
const LEVEL_RATIO: f64 = 1024.0;
/// Steps per unit, 2^54, as an exact float.
const STEPS_PER_UNIT: f64 = (1u64 << UNIT_BITS) as f64;

#[derive(Clone, Debug)]
pub struct Camera {
    /// The level the center is written on. It is a unit of measure and has no meaning outside the camera.
    pub level: Level,
    pub center: Position,
    /// Logical pixels per unit of `level`.
    pub zoom: f64,
    /// Logical pixels.
    pub viewport: [f64; 2],
}

impl Camera {
    pub fn new(center: Position, zoom: f64, viewport: [f64; 2]) -> Camera {
        let mut c = Camera {
            level: center.level,
            center,
            zoom,
            viewport,
        };
        c.normalize();
        c
    }

    /// Brings the zoom back into `[0.5, 2048)` by changing level. Going finer is exact. Going coarser drops the
    /// lowest ten bits of the center, which is under 2^-45 of a pixel.
    pub fn normalize(&mut self) {
        assert!(
            self.zoom.is_finite() && self.zoom > 0.0,
            "zoom must be a positive finite number"
        );
        while self.zoom >= ZOOM_MAX {
            self.level = Level(self.level.0 + 1);
            self.center = self.center.at_level(self.level);
            self.zoom /= LEVEL_RATIO;
        }
        while self.zoom < ZOOM_MIN {
            self.level = Level(self.level.0 - 1);
            self.center = self.center.at_coarser_floor(self.level);
            self.zoom *= LEVEL_RATIO;
        }
    }

    /// Moves the view by a number of screen pixels.
    pub fn pan_by_px(&mut self, dx: f64, dy: f64) {
        let d = Delta {
            level: self.level,
            dx: self.px_to_steps(dx),
            dy: self.px_to_steps(dy),
        };
        self.center = self.center.add_delta(&d);
    }

    /// Multiplies the zoom by `factor`, keeping the point under `focus_px` (measured from the top left of the
    /// viewport) where it is on screen.
    pub fn zoom_about(&mut self, focus_px: [f64; 2], factor: f64) {
        assert!(
            factor.is_finite() && factor > 0.0,
            "factor must be a positive finite number"
        );
        let z0 = self.zoom;
        let z1 = z0 * factor;
        // The focus point is d units from the center. After the change it must still be d * z0 pixels away, so the
        // center moves by d * (1 - z0 / z1).
        let keep = 1.0 - z0 / z1;
        let fx = (focus_px[0] - self.viewport[0] / 2.0) * keep;
        let fy = (focus_px[1] - self.viewport[1] / 2.0) * keep;
        self.pan_by_px(fx, fy);
        self.zoom = z1;
        self.normalize();
    }

    /// Exact jump to a position at a given zoom on that position's level.
    pub fn set(&mut self, center: Position, zoom: f64) {
        self.level = center.level;
        self.center = center;
        self.zoom = zoom;
        self.normalize();
    }

    /// Where a position lands on screen, in pixels from the top left of the viewport. `None` when it is too far
    /// away to be a number.
    pub fn project(&self, p: &Position) -> Option<[f64; 2]> {
        let [rx, ry] = p.sub(&self.center).to_f64(self.level);
        match (rx, ry) {
            (Rel::Near(x), Rel::Near(y)) => Some([
                x * self.zoom + self.viewport[0] / 2.0,
                y * self.zoom + self.viewport[1] / 2.0,
            ]),
            _ => None,
        }
    }

    /// Base-2 logarithm of the total scale: pixels per level-0 unit. Used only for display and tests.
    pub fn log2_scale(&self) -> f64 {
        self.zoom.log2() + f64::from(self.level.0) * f64::from(LEVEL_SHIFT)
    }

    fn px_to_steps(&self, px: f64) -> Axis {
        // At most a few thousand units: the product is far inside the range of an i128.
        let steps = (px / self.zoom * STEPS_PER_UNIT).round_ties_even();
        Axis::from_i128(steps as i128)
    }
}
