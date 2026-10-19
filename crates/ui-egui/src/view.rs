use egui::{Pos2, Rect, Vec2, pos2, vec2};
use laserfoundry_geom::Point;

pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 400.0;

/// Maps workspace mm (y down) to screen points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Screen position of workspace (0, 0).
    pub origin: Pos2,
    /// Screen points per mm.
    pub zoom: f32,
}

impl View {
    /// Fits a `w`×`h` mm bed into `canvas` with a margin.
    pub fn fit(canvas: Rect, w: f64, h: f64) -> View {
        let margin = 24.0;
        let zx = (canvas.width() - 2.0 * margin) / (w as f32).max(1.0);
        let zy = (canvas.height() - 2.0 * margin) / (h as f32).max(1.0);
        let zoom = zx.min(zy).clamp(MIN_ZOOM, MAX_ZOOM);
        let size = vec2(w as f32, h as f32) * zoom;
        View { origin: canvas.center() - size / 2.0, zoom }
    }

    pub fn to_screen(self, p: Point) -> Pos2 {
        pos2(self.origin.x + p.x as f32 * self.zoom, self.origin.y + p.y as f32 * self.zoom)
    }

    pub fn to_world(self, s: Pos2) -> Point {
        Point::new(f64::from((s.x - self.origin.x) / self.zoom), f64::from((s.y - self.origin.y) / self.zoom))
    }

    pub fn pan(&mut self, delta: Vec2) {
        self.origin += delta;
    }

    /// Zooms by `factor` keeping the world point under `anchor` fixed.
    pub fn zoom_at(&mut self, anchor: Pos2, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = new_zoom / self.zoom;
        self.origin = anchor + (self.origin - anchor) * k;
        self.zoom = new_zoom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0))
    }

    #[test]
    fn round_trip() {
        let v = View::fit(canvas(), 400.0, 400.0);
        let p = Point::new(123.4, 56.7);
        let q = v.to_world(v.to_screen(p));
        assert!((p.x - q.x).abs() < 1e-3 && (p.y - q.y).abs() < 1e-3);
    }

    #[test]
    fn fit_centres_and_fits_the_bed() {
        let v = View::fit(canvas(), 400.0, 400.0);
        let tl = v.to_screen(Point::new(0.0, 0.0));
        let br = v.to_screen(Point::new(400.0, 400.0));
        assert!(tl.x >= canvas().min.x && br.x <= canvas().max.x && tl.y >= canvas().min.y && br.y <= canvas().max.y);
        assert!((((tl.x + br.x) / 2.0) - canvas().center().x).abs() < 0.5);
    }

    #[test]
    fn zoom_keeps_the_anchor_fixed_and_clamps() {
        let mut v = View::fit(canvas(), 400.0, 400.0);
        let anchor = pos2(300.0, 300.0);
        let before = v.to_world(anchor);
        v.zoom_at(anchor, 2.0);
        let after = v.to_world(anchor);
        assert!((before.x - after.x).abs() < 1e-3 && (before.y - after.y).abs() < 1e-3);
        for _ in 0..50 {
            v.zoom_at(anchor, 10.0);
        }
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_at(anchor, f32::NAN);
        v.zoom_at(anchor, -1.0);
        assert_eq!(v.zoom, MAX_ZOOM);
    }

    #[test]
    fn degenerate_beds_do_not_divide_by_zero() {
        let v = View::fit(canvas(), 0.0, 0.0);
        assert!(v.zoom.is_finite() && v.origin.x.is_finite());
    }
}
