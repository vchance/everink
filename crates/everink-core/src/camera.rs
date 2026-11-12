//! The camera: an exact center, and a zoom held in a fixed range by changing level.
//!
//! The camera is local state. Its zoom is a float and may drift; its center is exact. `normalize` keeps the zoom in
//! `[0.5, 2048)` pixels per unit. The range is wider than one level (1024) on purpose: after a level change the
//! zoom must move by a factor of four before the level changes back, so a camera resting at a boundary does not
//! change level on every frame.
//!
//! Numbers from outside (pixels, zoom factors) are checked here. A value that is not finite, or is too large to be
//! a gesture, is refused with an error and changes nothing. The fields are private so that every change goes
//! through those checks.

use crate::{Axis, Delta, LEVEL_SHIFT, Level, Position, Rel, UNIT_BITS};

pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2048.0;
/// The most one pan may move, in pixels. Far more than any gesture; it keeps the step count inside an `i128`.
pub const MAX_PAN_PX: f64 = 1e12;
/// The most one zoom call may change the scale by, either way: 2^10, one level.
pub const MAX_ZOOM_FACTOR: f64 = 1024.0;
const LEVEL_RATIO: f64 = 1024.0;
/// Steps per unit, 2^54, as an exact float.
const STEPS_PER_UNIT: f64 = (1u64 << UNIT_BITS) as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraError {
    /// A number was NaN or infinite.
    NotFinite,
    /// A pan beyond `MAX_PAN_PX`, a zoom factor outside `[1/1024, 1024]`, a zoom that is not positive, or a
    /// negative viewport size.
    OutOfRange,
}

#[derive(Clone, Debug)]
pub struct Camera {
    /// The level the center is written on. It is a unit of measure and has no meaning outside the camera.
    level: Level,
    center: Position,
    /// Logical pixels per unit of `level`, in `[0.5, 2048)`.
    zoom: f64,
    /// Logical pixels.
    viewport: [f64; 2],
}

fn check_zoom(zoom: f64) -> Result<(), CameraError> {
    if !zoom.is_finite() {
        return Err(CameraError::NotFinite);
    }
    if zoom <= 0.0 {
        return Err(CameraError::OutOfRange);
    }
    Ok(())
}

fn check_viewport(viewport: [f64; 2]) -> Result<(), CameraError> {
    if !(viewport[0].is_finite() && viewport[1].is_finite()) {
        return Err(CameraError::NotFinite);
    }
    if viewport[0] < 0.0 || viewport[1] < 0.0 {
        return Err(CameraError::OutOfRange);
    }
    Ok(())
}

impl Camera {
    /// A camera at `center`. `zoom` is pixels per unit of the center's level; any positive finite value is
    /// accepted and brought into range by changing level.
    pub fn new(center: Position, zoom: f64, viewport: [f64; 2]) -> Result<Camera, CameraError> {
        check_zoom(zoom)?;
        check_viewport(viewport)?;
        let mut c = Camera {
            level: center.level,
            center,
            zoom,
            viewport,
        };
        c.normalize();
        Ok(c)
    }

    /// The level the center is written on.
    pub fn level(&self) -> Level {
        self.level
    }

    /// The point at the middle of the view. Exact.
    pub fn center(&self) -> &Position {
        &self.center
    }

    /// Logical pixels per unit of `level()`, in `[0.5, 2048)`.
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// The size of the view in logical pixels.
    pub fn viewport(&self) -> [f64; 2] {
        self.viewport
    }

    /// Changes the size of the view (a window resize). The center stays where it is.
    pub fn set_viewport(&mut self, viewport: [f64; 2]) -> Result<(), CameraError> {
        check_viewport(viewport)?;
        self.viewport = viewport;
        Ok(())
    }

    /// Brings the zoom back into `[0.5, 2048)` by changing level. Going finer is exact. Going coarser drops the
    /// lowest ten bits of the center, which is under 2^-45 of a pixel.
    fn normalize(&mut self) {
        debug_assert!(self.zoom.is_finite() && self.zoom > 0.0);
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
    pub fn pan_by_px(&mut self, dx: f64, dy: f64) -> Result<(), CameraError> {
        let d = Delta {
            level: self.level,
            dx: self.px_to_steps(dx)?,
            dy: self.px_to_steps(dy)?,
        };
        self.center = self.center.add_delta(&d);
        Ok(())
    }

    /// Multiplies the zoom by `factor`, keeping the point under `focus_px` (measured from the top left of the
    /// viewport) where it is on screen. One call changes the scale by at most 1024 either way.
    pub fn zoom_about(&mut self, focus_px: [f64; 2], factor: f64) -> Result<(), CameraError> {
        if !(factor.is_finite() && focus_px[0].is_finite() && focus_px[1].is_finite()) {
            return Err(CameraError::NotFinite);
        }
        if !(1.0 / MAX_ZOOM_FACTOR..=MAX_ZOOM_FACTOR).contains(&factor) {
            return Err(CameraError::OutOfRange);
        }
        let z0 = self.zoom;
        let z1 = z0 * factor; // z0 < 2048 and factor <= 1024, so this is finite and positive
        // The focus point is d units from the center. After the change it must still be d * z0 pixels away, so the
        // center moves by d * (1 - z0 / z1) units. `pan_by_px` takes pixels at the old zoom.
        let keep = 1.0 - z0 / z1;
        let fx = (focus_px[0] - self.viewport[0] / 2.0) * keep;
        let fy = (focus_px[1] - self.viewport[1] / 2.0) * keep;
        self.pan_by_px(fx, fy)?;
        self.zoom = z1;
        self.normalize();
        Ok(())
    }

    /// Exact jump to a position at a given zoom on that position's level. This is what "go to" a saved place, a
    /// person or home uses: it lands exactly, whatever the camera did before.
    pub fn set(&mut self, center: Position, zoom: f64) -> Result<(), CameraError> {
        check_zoom(zoom)?;
        self.level = center.level;
        self.center = center;
        self.zoom = zoom;
        self.normalize();
        Ok(())
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

    /// Base-2 logarithm of the total scale: pixels per level-0 unit. For display only; it uses a library function
    /// and is not part of any shared result.
    #[allow(clippy::disallowed_methods)] // a display value, never part of a shared result
    pub fn log2_scale(&self) -> f64 {
        self.zoom.log2() + f64::from(self.level.0) * f64::from(LEVEL_SHIFT)
    }

    fn px_to_steps(&self, px: f64) -> Result<Axis, CameraError> {
        if !px.is_finite() {
            return Err(CameraError::NotFinite);
        }
        if px.abs() > MAX_PAN_PX {
            return Err(CameraError::OutOfRange);
        }
        // At most 1e12 pixels at half a pixel per unit is 2e12 units, about 2^95 steps: inside an i128.
        let steps = (px / self.zoom * STEPS_PER_UNIT).round_ties_even();
        Ok(Axis::from_i128(steps as i128))
    }
}
