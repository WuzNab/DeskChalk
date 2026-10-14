//! The desktop icon grid: one dot per icon slot, box edges snap between dots.

use crate::geom::{Pt, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// A cell corner (monitor-relative pixels). Cell boundaries are `origin + k * cell`.
    pub origin: Pt,
    /// Cell size, i.e. the desktop icon spacing.
    pub cell: Pt,
    /// Where icons can live (the monitor's work area, monitor-relative).
    pub area: Rect,
}

impl Grid {
    pub fn snap_x(&self, x: f32) -> f32 {
        self.origin.x + ((x - self.origin.x) / self.cell.x).round() * self.cell.x
    }

    pub fn snap_y(&self, y: f32) -> f32 {
        self.origin.y + ((y - self.origin.y) / self.cell.y).round() * self.cell.y
    }

    pub fn floor_x(&self, x: f32) -> f32 {
        self.origin.x + ((x - self.origin.x) / self.cell.x).floor() * self.cell.x
    }

    pub fn floor_y(&self, y: f32) -> f32 {
        self.origin.y + ((y - self.origin.y) / self.cell.y).floor() * self.cell.y
    }

    pub fn ceil_x(&self, x: f32) -> f32 {
        self.origin.x + ((x - self.origin.x) / self.cell.x).ceil() * self.cell.x
    }

    pub fn ceil_y(&self, y: f32) -> f32 {
        self.origin.y + ((y - self.origin.y) / self.cell.y).ceil() * self.cell.y
    }

    /// How many icon slots a rect spans (rounded).
    pub fn cells_in(&self, r: &Rect) -> (i32, i32) {
        (
            (r.w / self.cell.x).round().max(0.0) as i32,
            (r.h / self.cell.y).round().max(0.0) as i32,
        )
    }

    /// Grid corners inside the work area that fall within `visible`. These are exactly the
    /// points box edges snap to; icons sit in the squares between them.
    pub fn dots(&self, visible: &Rect) -> Vec<Pt> {
        let (cw, ch) = (self.cell.x, self.cell.y);
        if cw < 4.0 || ch < 4.0 {
            return Vec::new();
        }
        let eps = 0.5;
        let k0 = ((self.area.x - self.origin.x - eps) / cw).ceil() as i32;
        let k1 = ((self.area.right() - self.origin.x + eps) / cw).floor() as i32;
        let j0 = ((self.area.y - self.origin.y - eps) / ch).ceil() as i32;
        let j1 = ((self.area.bottom() - self.origin.y + eps) / ch).floor() as i32;
        let mut out = Vec::new();
        for j in j0..=j1 {
            let y = self.origin.y + j as f32 * ch;
            if y < visible.y - 8.0 || y > visible.bottom() + 8.0 {
                continue;
            }
            for k in k0..=k1 {
                let x = self.origin.x + k as f32 * cw;
                if x >= visible.x - 8.0 && x <= visible.right() + 8.0 {
                    out.push(Pt::new(x, y));
                }
            }
        }
        out
    }
}

/// Finds the most common offset of `values` modulo `period` (icons aligned to the
/// grid all share it). Returns None when there are no samples.
pub fn dominant_phase(values: impl Iterator<Item = f32>, period: f32) -> Option<f32> {
    if period < 1.0 {
        return None;
    }
    let buckets = period.ceil() as usize + 1;
    let mut counts = vec![0u32; buckets];
    let mut any = false;
    for v in values {
        let r = v.rem_euclid(period).round() as usize % buckets;
        counts[r] += 1;
        any = true;
    }
    if !any {
        return None;
    }
    // Neighbouring buckets vote together so off-by-one pixel positions still agree.
    let n = counts.len();
    (0..n)
        .max_by_key(|&i| counts[(i + n - 1) % n] + 2 * counts[i] + counts[(i + 1) % n])
        .map(|i| i as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid {
            origin: Pt::new(2.0, 2.0),
            cell: Pt::new(75.0, 100.0),
            area: Rect::new(0.0, 0.0, 400.0, 300.0),
        }
    }

    #[test]
    fn snapping() {
        let g = grid();
        assert_eq!(g.snap_x(39.0), 2.0);
        assert_eq!(g.snap_x(40.0), 77.0);
        assert_eq!(g.floor_x(150.0), 77.0);
        assert_eq!(g.ceil_x(150.0), 152.0);
    }

    #[test]
    fn dots_on_cell_corners_inside_area() {
        let g = grid();
        let dots = g.dots(&g.area);
        // Corners at x = 2, 77, ..., 377 (6) and y = 2, 102, 202 (3).
        assert_eq!(dots.len(), 6 * 3);
        assert_eq!(dots[0], Pt::new(2.0, 2.0));
        assert_eq!(*dots.last().unwrap(), Pt::new(377.0, 202.0));
    }

    #[test]
    fn phase_detection() {
        let xs = [2.0, 77.0, 152.0, 153.0, 30.0];
        assert_eq!(dominant_phase(xs.into_iter(), 75.0), Some(2.0));
        assert_eq!(dominant_phase(std::iter::empty(), 75.0), None);
    }
}
