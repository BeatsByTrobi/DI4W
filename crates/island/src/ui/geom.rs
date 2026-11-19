//! Points, sizes, rectangles and vector paths, in device-independent units (WPF's DIPs, Apple's points).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

impl Size {
    pub const fn new(w: f64, h: f64) -> Self {
        Self { w, h }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.w && p.y < self.y + self.h
    }

    /// Smaller by the thickness on each side (never negative).
    pub fn deflate(&self, t: Thickness) -> Rect {
        Rect::new(self.x + t.left, self.y + t.top, (self.w - t.left - t.right).max(0.0), (self.h - t.top - t.bottom).max(0.0))
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

}

/// WPF's `Thickness`: margins and paddings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thickness {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Thickness {
    pub const fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self { left, top, right, bottom }
    }

    pub const fn all(v: f64) -> Self {
        Self::new(v, v, v, v)
    }

    pub fn horizontal(&self) -> f64 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f64 {
        self.top + self.bottom
    }
}

/// One step of a vector outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    /// Elliptical arc to the point: radius, large arc, clockwise.
    Arc(Point, f64, bool, bool),
    /// Closes the current figure.
    Close,
}

/// A vector outline (figures of lines and curves), drawn filled and/or stroked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
}

impl Path {
    pub fn move_to(&mut self, x: f64, y: f64) -> &mut Self {
        self.segs.push(Seg::Move(Point::new(x, y)));
        self
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> &mut Self {
        self.segs.push(Seg::Line(Point::new(x, y)));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.segs.push(Seg::Close);
        self
    }

    /// The bounding box of the points (control points included, which is close enough for icon stretching).
    pub fn bounds(&self) -> Rect {
        let mut min = Point::new(f64::MAX, f64::MAX);
        let mut max = Point::new(f64::MIN, f64::MIN);
        let mut add = |p: &Point| {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        };
        for s in &self.segs {
            match s {
                Seg::Move(p) | Seg::Line(p) | Seg::Arc(p, ..) => add(p),
                Seg::Quad(a, b) => {
                    add(a);
                    add(b);
                }
                Seg::Cubic(a, b, c) => {
                    add(a);
                    add(b);
                    add(c);
                }
                Seg::Close => {}
            }
        }
        if min.x > max.x {
            return Rect::default();
        }
        Rect::new(min.x, min.y, max.x - min.x, max.y - min.y)
    }

    /// The same outline scaled by `s` and moved by (`dx`, `dy`).
    pub fn transformed(&self, s: f64, dx: f64, dy: f64) -> Path {
        let t = |p: &Point| Point::new(p.x * s + dx, p.y * s + dy);
        Path {
            segs: self
                .segs
                .iter()
                .map(|seg| match seg {
                    Seg::Move(p) => Seg::Move(t(p)),
                    Seg::Line(p) => Seg::Line(t(p)),
                    Seg::Quad(a, b) => Seg::Quad(t(a), t(b)),
                    Seg::Cubic(a, b, c) => Seg::Cubic(t(a), t(b), t(c)),
                    Seg::Arc(p, r, large, cw) => Seg::Arc(t(p), r * s, *large, *cw),
                    Seg::Close => Seg::Close,
                })
                .collect(),
        }
    }

    /// Reads WPF path markup with the commands the icons use: M, L, H, V, Z (absolute).
    pub fn parse(data: &str) -> Path {
        let mut path = Path::default();
        let mut tokens = data.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).peekable();
        let mut current = Point::default();
        let mut command = 'M';
        fn number<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> f64 {
            tokens.next().and_then(|t| t.parse().ok()).unwrap_or(0.0)
        }
        while let Some(&token) = tokens.peek() {
            if let Some(c) = token.chars().next().filter(|c| c.is_ascii_alphabetic()) {
                command = c.to_ascii_uppercase();
                tokens.next();
                if command == 'Z' {
                    path.segs.push(Seg::Close);
                    continue;
                }
            }
            match command {
                'M' => {
                    current = Point::new(number(&mut tokens), number(&mut tokens));
                    path.segs.push(Seg::Move(current));
                    command = 'L'; // further pairs after M are lines
                }
                'L' => {
                    current = Point::new(number(&mut tokens), number(&mut tokens));
                    path.segs.push(Seg::Line(current));
                }
                'H' => {
                    current.x = number(&mut tokens);
                    path.segs.push(Seg::Line(current));
                }
                'V' => {
                    current.y = number(&mut tokens);
                    path.segs.push(Seg::Line(current));
                }
                _ => {
                    tokens.next();
                }
            }
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_icon_markup() {
        let pause = Path::parse("M 0,0 H 4.5 V 15 H 0 Z M 8.5,0 H 13 V 15 H 8.5 Z");
        assert_eq!(pause.segs.len(), 10);
        assert_eq!(pause.bounds(), Rect::new(0.0, 0.0, 13.0, 15.0));
        let close = Path::parse("M 0,0 L 12,12 M 12,0 L 0,12");
        assert_eq!(close.segs, vec![
            Seg::Move(Point::new(0.0, 0.0)),
            Seg::Line(Point::new(12.0, 12.0)),
            Seg::Move(Point::new(12.0, 0.0)),
            Seg::Line(Point::new(0.0, 12.0)),
        ]);
    }
}
