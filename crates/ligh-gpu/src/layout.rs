//! Simulator.app-style device chrome: chassis + screen + side buttons.
//!
//! Window coordinates are logical points, origin top-left (winit). The screen
//! rect is the only region that maps to the guest (0..1 → IndigoHID points).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DeviceLayout {
    pub chassis: Rect,
    pub screen: Rect,
    pub screen_radius: f64,
    pub chassis_radius: f64,
    pub power: Rect,
    pub vol_up: Rect,
    pub vol_down: Rect,
}

const PAD: f64 = 16.0;

impl DeviceLayout {
    pub fn preferred_window_size(screen_pts: (f64, f64), tablet: bool) -> (f64, f64) {
        let (cw, ch) = chassis_pts(screen_pts, tablet);
        let max_h = 820.0;
        let fit = ((max_h - 2.0 * PAD) / ch).min(1.05).max(0.35);
        (cw * fit + 2.0 * PAD, ch * fit + 2.0 * PAD)
    }

    /// Fit a device body into `win_w` × `win_h` (logical points).
    pub fn in_window(win_w: f64, win_h: f64, screen_pts: (f64, f64), tablet: bool) -> Self {
        let (sw, _sh) = screen_pts;
        let (cw, ch) = chassis_pts(screen_pts, tablet);
        let bezel = bezel_pts(tablet);
        let fit = ((win_w - 2.0 * PAD).max(1.0) / cw).min((win_h - 2.0 * PAD).max(1.0) / ch);
        let fit = fit.max(0.05);

        let chassis = Rect {
            x: (win_w - cw * fit) * 0.5,
            y: (win_h - ch * fit) * 0.5,
            w: cw * fit,
            h: ch * fit,
        };
        let b = bezel * fit;
        let screen = Rect {
            x: chassis.x + b,
            y: chassis.y + b,
            w: chassis.w - 2.0 * b,
            h: chassis.h - 2.0 * b,
        };

        let btn_w = (if tablet { 5.0 } else { 4.0 }) * fit;
        let btn_h = (if tablet { 40.0 } else { 32.0 }) * fit;
        let gap = 8.0 * fit;
        let power_h = btn_h * 1.7;

        Self {
            chassis,
            screen,
            screen_radius: screen_radius_pts(tablet, sw) * fit,
            chassis_radius: (screen_radius_pts(tablet, sw) + bezel * 0.55) * fit,
            power: Rect {
                x: chassis.x + chassis.w - btn_w - fit,
                y: chassis.y + chassis.h * 0.22,
                w: btn_w,
                h: power_h,
            },
            vol_up: Rect {
                x: chassis.x + fit,
                y: chassis.y + chassis.h * 0.20,
                w: btn_w,
                h: btn_h,
            },
            vol_down: Rect {
                x: chassis.x + fit,
                y: chassis.y + chassis.h * 0.20 + btn_h + gap,
                w: btn_w,
                h: btn_h,
            },
        }
    }

    /// Map window coords to guest 0..1. `None` = bezel / outside the glass.
    pub fn hit_screen(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !self.screen.contains(x, y) {
            return None;
        }
        let u = ((x - self.screen.x) / self.screen.w).clamp(0.0, 1.0);
        let v = ((y - self.screen.y) / self.screen.h).clamp(0.0, 1.0);
        Some((u, v))
    }
}

fn bezel_pts(tablet: bool) -> f64 {
    if tablet { 18.0 } else { 14.0 }
}

fn screen_radius_pts(tablet: bool, screen_w: f64) -> f64 {
    if tablet {
        12.0 * (screen_w / 834.0)
    } else {
        55.0 * (screen_w / 393.0)
    }
}

fn chassis_pts(screen_pts: (f64, f64), tablet: bool) -> (f64, f64) {
    let b = bezel_pts(tablet);
    (screen_pts.0 + 2.0 * b, screen_pts.1 + 2.0 * b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_click_is_guest_center() {
        let layout = DeviceLayout::in_window(457.0, 916.0, (393.0, 852.0), false);
        let cx = layout.screen.x + layout.screen.w * 0.5;
        let cy = layout.screen.y + layout.screen.h * 0.5;
        let (u, v) = layout.hit_screen(cx, cy).unwrap();
        assert!((u - 0.5).abs() < 0.01);
        assert!((v - 0.5).abs() < 0.01);
    }

    #[test]
    fn bezel_is_not_the_screen() {
        let layout = DeviceLayout::in_window(457.0, 916.0, (393.0, 852.0), false);
        assert!(layout.hit_screen(layout.chassis.x + 1.0, layout.chassis.y + 1.0).is_none());
    }

    #[test]
    fn screen_keeps_phone_aspect() {
        let layout = DeviceLayout::in_window(457.0, 916.0, (393.0, 852.0), false);
        let aspect = layout.screen.w / layout.screen.h;
        assert!((aspect - 393.0 / 852.0).abs() < 0.002);
    }
}
