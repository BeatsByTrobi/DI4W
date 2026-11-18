using System.Windows;
using System.Windows.Media;

namespace DynamicIsland.UI;

/// <summary>
/// The island's outline: a rounded rectangle with Apple's continuous ("squircle") corners, the curve iOS uses for
/// the Dynamic Island and every other rounded shape. A plain circular arc meets the straight edge abruptly; the
/// continuous corner eases in over about 1.53× the radius. When there isn't room for that (a capsule, whose radius
/// is half its height) the corner blends smoothly back into a circular arc, so the shape never jumps while it animates.
/// </summary>
public static class IslandShape
{
    /// <summary>How far Apple's continuous corner reaches along each edge, as a multiple of the radius.</summary>
    private const double Reach = 1.52866483;

    // One corner (top-right) as three cubic segments, in units of the radius: (u, v) = (distance from the right
    // edge, distance from the top edge). Apple's points (as measured from UIBezierPath's continuous corners):
    private static readonly Point[] Continuous =
    {
        new(1.52866483, 0), new(1.08849323, 0), new(0.86840689, 0), new(0.66993427, 0.06549600),
        new(0.63149399, 0.07491100), new(0.37282392, 0.16905899), new(0.16905899, 0.37282392), new(0.07491100, 0.63149399),
        new(0.06549600, 0.66993427), new(0, 0.86840689), new(0, 1.08849323), new(0, 1.52866483),
    };

    // The same corner as a plain circular arc, split at the same places (20° and 70°) so the two can be blended.
    private static readonly Point[] Circular = CircularCorner();

    /// <summary>The outline at (0,0) with the given size and corner radius (clamped to fit).</summary>
    public static Geometry Rounded(double width, double height, double radius)
    {
        var w = Math.Max(0, width);
        var h = Math.Max(0, height);
        var limit = Math.Min(w, h) / 2;
        var r = Math.Clamp(radius, 0, limit);

        var geometry = new StreamGeometry();
        if (r < 0.01)
        {
            using var c = geometry.Open();
            c.BeginFigure(new Point(0, 0), true, true);
            c.PolyLineTo(new[] { new Point(w, 0), new Point(w, h), new Point(0, h) }, false, false);
        }
        else
        {
            // 1 = full continuous corner; 0 = circular (the capsule's ends).
            var smooth = Math.Clamp((limit / r - 1) / (Reach - 1), 0, 1);
            var corner = new Point[Continuous.Length];
            for (var i = 0; i < corner.Length; i++)
            {
                corner[i] = new Point(Lerp(Circular[i].X, Continuous[i].X, smooth) * r, Lerp(Circular[i].Y, Continuous[i].Y, smooth) * r);
            }

            using var c = geometry.Open();
            // Clockwise from the top-left corner's end: top edge, then each corner.
            var corners = new Func<Point, Point>[]
            {
                p => new Point(w - p.X, p.Y),     // top-right
                p => new Point(w - p.Y, h - p.X), // bottom-right
                p => new Point(p.X, h - p.Y),     // bottom-left
                p => new Point(p.Y, p.X),         // top-left
            };
            c.BeginFigure(corners[3](corner[^1]), true, true);
            foreach (var map in corners)
            {
                c.LineTo(map(corner[0]), true, false);
                c.BezierTo(map(corner[1]), map(corner[2]), map(corner[3]), true, true);
                c.LineTo(map(corner[4]), true, true);
                c.BezierTo(map(corner[5]), map(corner[6]), map(corner[7]), true, true);
                c.LineTo(map(corner[8]), true, true);
                c.BezierTo(map(corner[9]), map(corner[10]), map(corner[11]), true, true);
            }
        }
        geometry.Freeze();
        return geometry;
    }

    private static double Lerp(double a, double b, double t) => a + (b - a) * t;

    private static Point[] CircularCorner()
    {
        // Centre at (1,1); the point at angle φ is (1 - sin φ, 1 - cos φ), going from the top edge (φ=0) to the side (φ=90°).
        Point At(double deg) => new(1 - Math.Sin(deg * Math.PI / 180), 1 - Math.Cos(deg * Math.PI / 180));
        Vector Tangent(double deg) => new(-Math.Cos(deg * Math.PI / 180), Math.Sin(deg * Math.PI / 180));

        var points = new List<Point>();
        foreach (var (a, b) in new[] { (0.0, 20.0), (20.0, 70.0), (70.0, 90.0) })
        {
            var k = 4.0 / 3 * Math.Tan((b - a) * Math.PI / 180 / 4); // handle length for a cubic arc of this angle
            // Same layout as Apple's 12 points: each segment starts where the last ended (the short straight
            // piece between them has no length here).
            points.Add(At(a));
            points.Add(At(a) + Tangent(a) * k);
            points.Add(At(b) - Tangent(b) * k);
            points.Add(At(b));
        }
        return points.ToArray();
    }
}
