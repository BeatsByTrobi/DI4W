//! The island's outline: a rounded rectangle with Apple's continuous ("squircle") corners, the curve iOS uses for
//! the Dynamic Island and every other rounded shape. A plain circular arc meets the straight edge abruptly; the
//! continuous corner eases in over about 1.53× the radius. When there isn't room for that (a capsule, whose radius
//! is half its height) the corner blends smoothly back into a circular arc, so the shape never jumps while it
//! animates.

use super::geom::{Path, Point, Seg};
use std::sync::OnceLock;

/// How far Apple's continuous corner reaches along each edge, as a multiple of the radius.
const REACH: f64 = 1.528_664_83;

/// One corner (top-right) as three cubic segments, in units of the radius: (u, v) = (distance from the right
/// edge, distance from the top edge). Apple's points (as measured from UIBezierPath's continuous corners).
const CONTINUOUS: [(f64, f64); 12] = [
    (1.528_664_83, 0.0),
    (1.088_493_23, 0.0),
    (0.868_406_89, 0.0),
    (0.669_934_27, 0.065_496_00),
    (0.631_493_99, 0.074_911_00),
    (0.372_823_92, 0.169_058_99),
    (0.169_058_99, 0.372_823_92),
    (0.074_911_00, 0.631_493_99),
    (0.065_496_00, 0.669_934_27),
    (0.0, 0.868_406_89),
    (0.0, 1.088_493_23),
    (0.0, 1.528_664_83),
];

/// The same corner as a plain circular arc, split at the same places (20° and 70°) so the two can be blended.
fn circular() -> &'static [(f64, f64); 12] {
    static CIRCULAR: OnceLock<[(f64, f64); 12]> = OnceLock::new();
    CIRCULAR.get_or_init(|| {
        // Centre at (1,1); the point at angle φ is (1 - sin φ, 1 - cos φ), going from the top edge (φ=0) to the
        // side (φ=90°).
        let at = |deg: f64| {
            let r = deg.to_radians();
            (1.0 - r.sin(), 1.0 - r.cos())
        };
        let tangent = |deg: f64| {
            let r = deg.to_radians();
            (-r.cos(), r.sin())
        };
        let mut points = [(0.0, 0.0); 12];
        for (i, (a, b)) in [(0.0, 20.0), (20.0, 70.0), (70.0, 90.0)].into_iter().enumerate() {
            // Handle length for a cubic arc of this angle.
            let k = 4.0 / 3.0 * (f64::to_radians(b - a) / 4.0).tan();
            // Same layout as Apple's 12 points: each segment starts where the last ended (the short straight piece
            // between them has no length here).
            let (pa, ta, pb, tb) = (at(a), tangent(a), at(b), tangent(b));
            points[i * 4] = pa;
            points[i * 4 + 1] = (pa.0 + ta.0 * k, pa.1 + ta.1 * k);
            points[i * 4 + 2] = (pb.0 - tb.0 * k, pb.1 - tb.1 * k);
            points[i * 4 + 3] = pb;
        }
        points
    })
}

/// The outline at (0,0) with the given size and corner radius (clamped to fit).
pub fn rounded(width: f64, height: f64, radius: f64) -> Path {
    let w = width.max(0.0);
    let h = height.max(0.0);
    let limit = w.min(h) / 2.0;
    let r = radius.clamp(0.0, limit.max(0.0));

    let mut path = Path::default();
    if r < 0.01 {
        path.move_to(0.0, 0.0).line_to(w, 0.0).line_to(w, h).line_to(0.0, h).close();
        return path;
    }

    // 1 = full continuous corner; 0 = circular (the capsule's ends).
    let smooth = ((limit / r - 1.0) / (REACH - 1.0)).clamp(0.0, 1.0);
    let circular = circular();
    let corner: Vec<(f64, f64)> = (0..12)
        .map(|i| (lerp(circular[i].0, CONTINUOUS[i].0, smooth) * r, lerp(circular[i].1, CONTINUOUS[i].1, smooth) * r))
        .collect();

    // Clockwise from the top-left corner's end: top edge, then each corner.
    let corners: [&dyn Fn((f64, f64)) -> Point; 4] = [
        &|(x, y)| Point::new(w - x, y),     // top-right
        &|(x, y)| Point::new(w - y, h - x), // bottom-right
        &|(x, y)| Point::new(x, h - y),     // bottom-left
        &|(x, y)| Point::new(y, x),         // top-left
    ];
    path.segs.push(Seg::Move(corners[3](corner[11])));
    for map in corners {
        path.segs.push(Seg::Line(map(corner[0])));
        path.segs.push(Seg::Cubic(map(corner[1]), map(corner[2]), map(corner[3])));
        path.segs.push(Seg::Line(map(corner[4])));
        path.segs.push(Seg::Cubic(map(corner[5]), map(corner[6]), map(corner[7])));
        path.segs.push(Seg::Line(map(corner[8])));
        path.segs.push(Seg::Cubic(map(corner[9]), map(corner[10]), map(corner[11])));
    }
    path.segs.push(Seg::Close);
    path
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(path: &Path) -> Vec<Point> {
        path.segs
            .iter()
            .flat_map(|s| match *s {
                Seg::Move(p) | Seg::Line(p) => vec![p],
                Seg::Cubic(a, b, c) => vec![a, b, c],
                _ => vec![],
            })
            .collect()
    }

    #[test]
    fn stays_inside_its_box() {
        for (w, h, r) in [(126.0, 37.0, 18.5), (371.0, 160.0, 44.0), (230.0, 37.0, 10.0), (20.0, 20.0, 50.0)] {
            for p in points(&rounded(w, h, r)) {
                assert!(p.x >= -1e-9 && p.x <= w + 1e-9 && p.y >= -1e-9 && p.y <= h + 1e-9, "{w}x{h} r{r}: {p:?}");
            }
        }
    }

    #[test]
    fn a_capsule_has_circular_ends() {
        // Radius = half the height: the corner is a quarter circle, so its middle point sits on the circle.
        let r = 18.5;
        let path = rounded(126.0, 37.0, r);
        let mid = points(&path)[4]; // the top-right corner's 20° point
        let (cx, cy) = (126.0 - r, r);
        let distance = ((mid.x - cx).powi(2) + (mid.y - cy).powi(2)).sqrt();
        assert!((distance - r).abs() < 1e-6, "{distance}");
    }

    #[test]
    fn a_roomy_corner_is_continuous() {
        // Plenty of room: Apple's corner, which starts 1.53 radii before the corner.
        let path = rounded(371.0, 160.0, 44.0);
        let start = points(&path)[1]; // where the top edge meets the top-right corner
        assert!((start.x - (371.0 - REACH * 44.0)).abs() < 1e-6);
        assert_eq!(start.y, 0.0);
    }

    #[test]
    fn no_radius_is_a_rectangle() {
        assert_eq!(rounded(10.0, 5.0, 0.0).segs.len(), 5);
    }
}
