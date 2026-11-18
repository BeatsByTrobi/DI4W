//! A small retained element tree with WPF's layout rules, so the island's views can be written the way the C# app
//! built them (StackPanel, Grid, Border, TextBlock, Path, Button) and look the same.
//!
//! Views are plain functions that build an [`El`] tree from their current state each time the island is drawn.
//! [`El::layout`] measures and arranges it (desired sizes, margins, alignment, star columns, explicit sizes),
//! storing each element's rectangle; the renderer then paints it and the window hit-tests it for clicks, hover,
//! the mouse wheel and tooltips.

use super::colors::Color;
use super::geom::{Path, Point, Rect, Size, Thickness};
use super::text::{self, TextLayout};
use std::cell::RefCell;
use std::rc::Rc;

/// Something to do on a click.
pub type Action = Rc<dyn Fn()>;

/// Stable identity of a button across rebuilds (for hover and the press animation).
pub fn key(name: &str) -> u64 {
    // FNV-1a
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// WPF's default: fill the slot (centered when an explicit size makes the element smaller).
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Inherit,
    Hand,
    SizeNs,
}

/// A grid column width or row height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Auto,
    Px(f64),
    Star(f64),
}

/// Fill of a shape or box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Brush {
    Solid(Color),
    /// WPF's `LinearGradientBrush(top, bottom, 90)`: top to bottom across the element's box.
    Vertical(Color, Color),
    /// An image, filling the box like `Stretch.UniformToFill`.
    Image(u64),
}

impl From<Color> for Brush {
    fn from(c: Color) -> Self {
        Brush::Solid(c)
    }
}

/// The font of a text: the island's text font, the icon font, Segoe UI Symbol (weather symbols), or the menu's
/// Windows font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Text,
    Icons,
    Symbols,
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// One run of a text with its own size, weight and color.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub size: f64,
    pub weight: u16,
    pub color: Color,
}

/// A TextBlock: one or more runs in one font, trimmed with an ellipsis when it doesn't fit.
#[derive(Clone)]
pub struct Text {
    pub runs: Vec<Run>,
    pub font: Font,
    pub tabular: bool,
    pub wrap: bool,
    pub align: TextAlign,
    pub trim: bool,
    /// Made when the element is arranged; used to paint it.
    pub layout: RefCell<Option<TextLayout>>,
}

impl Text {
    /// The whole text (all runs).
    pub fn content(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

/// A vector shape, stretched uniformly into the element's box (WPF's `Path` with `Stretch.Uniform`).
#[derive(Clone)]
pub struct Shape {
    pub path: Rc<Path>,
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub thickness: f64,
    pub round_caps: bool,
    /// Stretch the path into the box (icons); otherwise draw it at its own coordinates.
    pub stretch: bool,
}

/// What an element is.
pub enum Kind {
    /// A box (WPF's Border or Grid with one cell): background, padding, children on top of each other.
    Boxed(Vec<El>),
    Text(Text),
    Shape(Shape),
    /// An ellipse outline inside the box (the track under a progress ring).
    Ellipse { stroke: Color, thickness: f64 },
    /// Circular progress arc from 12 o'clock, clockwise (the C#'s `Icons.Ring`).
    Ring { progress: f64, thickness: f64, color: Color },
    Stack { horizontal: bool, children: Vec<El> },
    Grid { cols: Vec<Len>, rows: Vec<Len>, cells: Vec<(usize, usize, El)> },
    /// One row of equal columns (WPF's UniformGrid with Rows=1 or Columns=n).
    Uniform { children: Vec<El> },
    /// A vertically scrolling list (the mouse wheel scrolls it; no scroll bar).
    Scroll { child: Box<El>, offset: Rc<std::cell::Cell<f64>> },
    /// Drawn by code (music bars, battery).
    Custom(Rc<dyn Fn(&mut dyn Paint, Rect)>),
}

/// The drawing operations custom elements use (implemented by the Direct2D renderer).
pub trait Paint {
    fn fill_rounded(&mut self, rect: Rect, radius: f64, brush: Brush);
    fn fill_path(&mut self, path: &Path, color: Color);
    fn stroke_path(&mut self, path: &Path, color: Color, thickness: f64, round_caps: bool);
}

/// A clickable element: WPF's `Button` with the island's style (dims to 0.8 on hover, shrinks to 0.88 while
/// pressed and springs back).
#[derive(Clone)]
pub struct ButtonSpec {
    pub key: u64,
    pub action: Action,
}

pub struct El {
    pub kind: Kind,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub max_height: Option<f64>,
    pub margin: Thickness,
    pub padding: Thickness,
    pub h: Align,
    pub v: Align,
    pub background: Option<Brush>,
    /// Corner radius (top-left, top-right, bottom-right, bottom-left).
    pub radius: [f64; 4],
    pub border: Option<(Color, f64)>,
    pub opacity: f64,
    pub collapsed: bool,
    /// Shown on hover, and read by screen readers and the Recent list.
    pub tooltip: Option<String>,
    /// Name for screen readers (icon-only buttons).
    pub name: Option<String>,
    pub button: Option<ButtonSpec>,
    /// A click that isn't a button (album art, a Recent row, the seek bar); gets the click position in the box.
    pub click: Option<Rc<dyn Fn(Point, Rect)>>,
    /// Mouse wheel: +1 up, -1 down.
    pub wheel: Option<Rc<dyn Fn(i32)>>,
    pub cursor: Cursor,
    /// Render scale around the center (a pulsing icon).
    pub scale: f64,
    /// Where layout put it, in the coordinates of the tree's root.
    pub rect: Rect,
    desired: Size,
    /// The size it wants without margins, before being clipped to the room it got (WPF arranges at this size).
    unclipped: Size,
}

impl El {
    pub fn new(kind: Kind) -> El {
        El {
            kind,
            width: None,
            height: None,
            max_height: None,
            margin: Thickness::default(),
            padding: Thickness::default(),
            h: Align::Stretch,
            v: Align::Stretch,
            background: None,
            radius: [0.0; 4],
            border: None,
            opacity: 1.0,
            collapsed: false,
            tooltip: None,
            name: None,
            button: None,
            click: None,
            wheel: None,
            cursor: Cursor::Inherit,
            scale: 1.0,
            rect: Rect::default(),
            desired: Size::default(),
            unclipped: Size::default(),
        }
    }

    /// An empty box (WPF's `Border` / `Grid`), with children layered on top of each other.
    pub fn boxed(children: Vec<El>) -> El {
        El::new(Kind::Boxed(children))
    }

    pub fn empty() -> El {
        El::boxed(Vec::new())
    }

    pub fn hstack(children: Vec<El>) -> El {
        El::new(Kind::Stack { horizontal: true, children })
    }

    pub fn vstack(children: Vec<El>) -> El {
        El::new(Kind::Stack { horizontal: false, children })
    }

    pub fn grid(cols: Vec<Len>, rows: Vec<Len>, cells: Vec<(usize, usize, El)>) -> El {
        El::new(Kind::Grid { cols, rows, cells })
    }

    pub fn uniform(children: Vec<El>) -> El {
        El::new(Kind::Uniform { children })
    }

    pub fn custom(draw: impl Fn(&mut dyn Paint, Rect) + 'static) -> El {
        El::new(Kind::Custom(Rc::new(draw)))
    }

    // ── builder helpers ──

    pub fn size(mut self, w: f64, h: f64) -> El {
        self.width = Some(w);
        self.height = Some(h);
        self
    }

    pub fn w(mut self, w: f64) -> El {
        self.width = Some(w);
        self
    }

    pub fn h(mut self, h: f64) -> El {
        self.height = Some(h);
        self
    }

    pub fn max_h(mut self, h: f64) -> El {
        self.max_height = Some(h);
        self
    }

    pub fn margin(mut self, l: f64, t: f64, r: f64, b: f64) -> El {
        self.margin = Thickness::new(l, t, r, b);
        self
    }

    pub fn margin_all(mut self, v: f64) -> El {
        self.margin = Thickness::all(v);
        self
    }

    pub fn padding(mut self, l: f64, t: f64, r: f64, b: f64) -> El {
        self.padding = Thickness::new(l, t, r, b);
        self
    }

    pub fn align(mut self, h: Align, v: Align) -> El {
        self.h = h;
        self.v = v;
        self
    }

    pub fn halign(mut self, h: Align) -> El {
        self.h = h;
        self
    }

    pub fn valign(mut self, v: Align) -> El {
        self.v = v;
        self
    }

    pub fn bg(mut self, brush: impl Into<Brush>) -> El {
        self.background = Some(brush.into());
        self
    }

    pub fn radius(mut self, r: f64) -> El {
        self.radius = [r; 4];
        self
    }

    pub fn radii(mut self, tl: f64, tr: f64, br: f64, bl: f64) -> El {
        self.radius = [tl, tr, br, bl];
        self
    }

    pub fn border(mut self, color: Color, thickness: f64) -> El {
        self.border = Some((color, thickness));
        self
    }

    pub fn opacity(mut self, o: f64) -> El {
        self.opacity = o;
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> El {
        self.collapsed = collapsed;
        self
    }

    pub fn tooltip(mut self, tip: impl Into<String>) -> El {
        self.tooltip = Some(tip.into());
        self
    }

    pub fn tooltip_opt(mut self, tip: Option<String>) -> El {
        self.tooltip = tip;
        self
    }

    pub fn named(mut self, name: impl Into<String>) -> El {
        self.name = Some(name.into());
        self
    }

    pub fn on_click(mut self, f: impl Fn(Point, Rect) + 'static) -> El {
        self.click = Some(Rc::new(f));
        self
    }

    pub fn on_wheel(mut self, f: impl Fn(i32) + 'static) -> El {
        self.wheel = Some(Rc::new(f));
        self
    }

    pub fn cursor(mut self, c: Cursor) -> El {
        self.cursor = c;
        self
    }

    pub fn scaled(mut self, s: f64) -> El {
        self.scale = s;
        self
    }

    /// The element's children, in reading order.
    pub fn children(&self) -> Vec<&El> {
        match &self.kind {
            Kind::Boxed(c) | Kind::Stack { children: c, .. } | Kind::Uniform { children: c } => c.iter().collect(),
            Kind::Grid { cells, .. } => cells.iter().map(|(_, _, e)| e).collect(),
            Kind::Scroll { child, .. } => vec![child.as_ref()],
            _ => Vec::new(),
        }
    }

    fn children_mut(&mut self) -> Vec<&mut El> {
        match &mut self.kind {
            Kind::Boxed(c) | Kind::Stack { children: c, .. } | Kind::Uniform { children: c } => c.iter_mut().collect(),
            Kind::Grid { cells, .. } => cells.iter_mut().map(|(_, _, e)| e).collect(),
            Kind::Scroll { child, .. } => vec![child.as_mut()],
            _ => Vec::new(),
        }
    }

    // ───────────────────────────── layout ─────────────────────────────

    /// Measures and arranges the tree into `rect` (as WPF does for an element given that slot).
    pub fn layout(&mut self, rect: Rect) {
        self.measure(Size::new(rect.w, rect.h));
        self.arrange(rect);
    }

    /// The size this element wants (margins included), given the room available. WPF's `Measure`.
    pub fn measure(&mut self, available: Size) -> Size {
        if self.collapsed {
            self.desired = Size::default();
            return self.desired;
        }
        let m = self.margin;
        let mut inner = Size::new((available.w - m.horizontal()).max(0.0), (available.h - m.vertical()).max(0.0));
        if let Some(w) = self.width {
            inner.w = w;
        }
        if let Some(h) = self.height {
            inner.h = h;
        }
        if let Some(max) = self.max_height {
            inner.h = inner.h.min(max);
        }
        let p = self.padding;
        let content_room = Size::new((inner.w - p.horizontal()).max(0.0), (inner.h - p.vertical()).max(0.0));
        let content = self.measure_content(content_room);
        let mut size = Size::new(content.w + p.horizontal(), content.h + p.vertical());
        if let Some(w) = self.width {
            size.w = w;
        }
        if let Some(h) = self.height {
            size.h = h;
        }
        if let Some(max) = self.max_height {
            size.h = size.h.min(max);
        }
        self.unclipped = size;
        // Like WPF, the parent sees at most the room it offered; the element still arranges at its full size.
        self.desired = Size::new((size.w + m.horizontal()).min(available.w), (size.h + m.vertical()).min(available.h));
        self.desired
    }

    fn measure_content(&mut self, room: Size) -> Size {
        match &mut self.kind {
            Kind::Boxed(children) => {
                let mut size = Size::default();
                for c in children {
                    let d = c.measure(room);
                    size.w = size.w.max(d.w);
                    size.h = size.h.max(d.h);
                }
                size
            }
            Kind::Text(t) => {
                let layout = text::layout(t, room.w, room.h);
                let size = layout.size();
                *t.layout.borrow_mut() = Some(layout);
                size
            }
            Kind::Stack { horizontal, children } => {
                let mut size = Size::default();
                for c in children {
                    if *horizontal {
                        let d = c.measure(Size::new(f64::INFINITY, room.h));
                        size.w += d.w;
                        size.h = size.h.max(d.h);
                    } else {
                        let d = c.measure(Size::new(room.w, f64::INFINITY));
                        size.h += d.h;
                        size.w = size.w.max(d.w);
                    }
                }
                size
            }
            Kind::Grid { cols, rows, cells } => {
                let (col_w, row_h) = grid_tracks(cols, rows, cells, room, true);
                let _ = &col_w;
                Size::new(
                    track_desired(cols, &col_w, cells, true),
                    track_desired(rows, &row_h, cells, false),
                )
            }
            Kind::Uniform { children } => {
                let n = children.len().max(1) as f64;
                let cell = Size::new(room.w / n, room.h);
                let mut max = Size::default();
                for c in children {
                    let d = c.measure(cell);
                    max.w = max.w.max(d.w);
                    max.h = max.h.max(d.h);
                }
                Size::new(max.w * n, max.h)
            }
            Kind::Scroll { child, .. } => {
                let d = child.measure(Size::new(room.w, f64::INFINITY));
                Size::new(d.w, d.h.min(room.h))
            }
            _ => Size::default(),
        }
    }

    /// Places the element in its slot (WPF's `Arrange`): margins, explicit sizes, alignment.
    pub fn arrange(&mut self, slot: Rect) {
        if self.collapsed {
            self.rect = Rect::new(slot.x, slot.y, 0.0, 0.0);
            return;
        }
        let m = self.margin;
        let client = slot.deflate(m);
        let wanted = self.unclipped;

        let mut size = Size::new(client.w, client.h);
        if self.h != Align::Stretch || size.w < wanted.w {
            size.w = wanted.w;
        }
        if self.v != Align::Stretch || size.h < wanted.h {
            size.h = wanted.h;
        }
        if let Some(w) = self.width {
            size.w = w;
        }
        if let Some(h) = self.height {
            size.h = h;
        }
        if let Some(max) = self.max_height {
            size.h = size.h.min(max);
        }

        let offset = |align: Align, room: f64, size: f64| match align {
            Align::Start => 0.0,
            Align::End => room - size,
            // Stretch behaves like Center when the element ends up smaller (explicit size), like Left when bigger.
            Align::Stretch if size > room => 0.0,
            _ => (room - size) / 2.0,
        };
        let x = client.x + offset(self.h, client.w, size.w);
        let y = client.y + offset(self.v, client.h, size.h);
        self.rect = Rect::new(x, y, size.w, size.h);
        let content = self.rect.deflate(self.padding);
        self.arrange_content(content);
    }

    fn arrange_content(&mut self, area: Rect) {
        match &mut self.kind {
            Kind::Boxed(children) => {
                for c in children {
                    c.arrange(area);
                }
            }
            Kind::Text(t) => {
                // Lay out again at the arranged width when it matters: trimming (the slot is narrower than the
                // text), wrapping, or centered / right-aligned text (aligned within the box).
                let needs = t.layout.borrow().as_ref().is_none_or(|l| {
                    let width_changed = (l.max_width - area.w).abs() > 0.01;
                    area.w + 0.01 < l.width || (width_changed && (t.wrap || t.align != TextAlign::Left))
                });
                if needs {
                    let max_h = t.layout.borrow().as_ref().map_or(f64::INFINITY, |l| l.max_height);
                    *t.layout.borrow_mut() = Some(text::layout(t, area.w + 0.01, max_h));
                }
            }
            Kind::Stack { horizontal, children } => {
                let mut pos = if *horizontal { area.x } else { area.y };
                for c in children {
                    if *horizontal {
                        let w = c.desired.w;
                        c.arrange(Rect::new(pos, area.y, w, area.h));
                        pos += w;
                    } else {
                        let h = c.desired.h;
                        c.arrange(Rect::new(area.x, pos, area.w, h));
                        pos += h;
                    }
                }
            }
            Kind::Grid { cols, rows, cells } => {
                let (col_w, row_h) = grid_tracks(cols, rows, cells, Size::new(area.w, area.h), false);
                let col_x = prefix(&col_w, area.x);
                let row_y = prefix(&row_h, area.y);
                for (r, c, el) in cells.iter_mut() {
                    let (r, c) = ((*r).min(row_h.len() - 1), (*c).min(col_w.len() - 1));
                    el.arrange(Rect::new(col_x[c], row_y[r], col_w[c], row_h[r]));
                }
            }
            Kind::Uniform { children } => {
                let n = children.len().max(1) as f64;
                let w = area.w / n;
                for (i, c) in children.iter_mut().enumerate() {
                    c.arrange(Rect::new(area.x + w * i as f64, area.y, w, area.h));
                }
            }
            Kind::Scroll { child, offset } => {
                let extent = child.desired.h;
                let max = (extent - area.h).max(0.0);
                let o = offset.get().clamp(0.0, max);
                offset.set(o);
                child.arrange(Rect::new(area.x, area.y - o, area.w, extent));
            }
            _ => {}
        }
    }

    // ───────────────────────────── hit testing ─────────────────────────────

    /// The elements under the point, outermost first (only visible ones).
    pub fn hits(&self, p: Point) -> Vec<&El> {
        let mut out = Vec::new();
        self.collect_hits(p, &mut out, None);
        out
    }

    fn collect_hits<'a>(&'a self, p: Point, out: &mut Vec<&'a El>, clip: Option<Rect>) {
        if self.collapsed || self.opacity <= 0.0 {
            return;
        }
        if let Some(c) = clip
            && !c.contains(p) {
                return;
            }
        if !self.rect.contains(p) && !matches!(self.kind, Kind::Boxed(_) | Kind::Stack { .. } | Kind::Grid { .. }) {
            return;
        }
        if self.rect.contains(p) {
            out.push(self);
        }
        let clip = if let Kind::Scroll { .. } = self.kind { Some(self.rect) } else { clip };
        for c in self.children() {
            c.collect_hits(p, out, clip);
        }
    }

    /// Moves every element by (dx, dy) (to place a laid-out tree somewhere else).
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.rect = self.rect.offset(dx, dy);
        for c in self.children_mut() {
            c.translate(dx, dy);
        }
    }
}

/// Column widths and row heights for a grid in the given room. `measuring`: children are measured too.
fn grid_tracks(cols: &[Len], rows: &[Len], cells: &mut [(usize, usize, El)], room: Size, measuring: bool) -> (Vec<f64>, Vec<f64>) {
    let cols: Vec<Len> = if cols.is_empty() { vec![Len::Star(1.0)] } else { cols.to_vec() };
    let rows: Vec<Len> = if rows.is_empty() { vec![Len::Star(1.0)] } else { rows.to_vec() };
    let clamp = |i: usize, n: usize| i.min(n - 1);

    // Pass 1: children in auto/pixel columns get their natural width (infinite for auto), so auto columns can be
    // sized; star columns get what's left.
    let mut col_w: Vec<f64> = cols.iter().map(|c| if let Len::Px(px) = c { *px } else { 0.0 }).collect();
    let mut row_h: Vec<f64> = rows.iter().map(|r| if let Len::Px(px) = r { *px } else { 0.0 }).collect();

    if measuring {
        for (r, c, el) in cells.iter_mut() {
            let (r, c) = (clamp(*r, rows.len()), clamp(*c, cols.len()));
            if matches!(cols[c], Len::Auto) {
                let h = if let Len::Px(px) = rows[r] { px } else { room.h };
                let d = el.measure(Size::new(f64::INFINITY, h));
                col_w[c] = col_w[c].max(d.w);
            }
        }
    } else {
        for (_, c, el) in cells.iter() {
            let c = clamp(*c, cols.len());
            if matches!(cols[c], Len::Auto) {
                col_w[c] = col_w[c].max(el.desired.w);
            }
        }
    }
    distribute(&cols, &mut col_w, room.w);

    if measuring {
        for (r, c, el) in cells.iter_mut() {
            let (r, c) = (clamp(*r, rows.len()), clamp(*c, cols.len()));
            if !matches!(cols[c], Len::Auto) {
                let h = if let Len::Px(px) = rows[r] { px } else { room.h };
                el.measure(Size::new(col_w[c], h));
            }
        }
    }
    for (r, _, el) in cells.iter() {
        let r = clamp(*r, rows.len());
        if matches!(rows[r], Len::Auto) {
            row_h[r] = row_h[r].max(el.desired.h);
        }
    }
    distribute(&rows, &mut row_h, room.h);
    if measuring {
        // Star tracks with infinite room: as big as their content.
        for (r, c, el) in cells.iter() {
            let (r, c) = (clamp(*r, rows.len()), clamp(*c, cols.len()));
            if room.w.is_infinite() && matches!(cols[c], Len::Star(_)) {
                col_w[c] = col_w[c].max(el.desired.w);
            }
            if room.h.is_infinite() && matches!(rows[r], Len::Star(_)) {
                row_h[r] = row_h[r].max(el.desired.h);
            }
        }
    }
    (col_w, row_h)
}

/// Gives star tracks their share of what the fixed and auto tracks leave.
fn distribute(tracks: &[Len], sizes: &mut [f64], room: f64) {
    if !room.is_finite() {
        return;
    }
    let fixed: f64 = tracks.iter().zip(sizes.iter()).filter(|(t, _)| !matches!(t, Len::Star(_))).map(|(_, s)| *s).sum();
    let stars: f64 = tracks.iter().map(|t| if let Len::Star(s) = t { *s } else { 0.0 }).sum();
    if stars <= 0.0 {
        return;
    }
    let left = (room - fixed).max(0.0);
    for (t, s) in tracks.iter().zip(sizes.iter_mut()) {
        if let Len::Star(weight) = t {
            *s = left * weight / stars;
        }
    }
}

/// The grid's desired width (or height): fixed and auto tracks, plus what children in star tracks want.
fn track_desired(tracks: &[Len], sizes: &[f64], cells: &[(usize, usize, El)], cols: bool) -> f64 {
    let tracks: Vec<Len> = if tracks.is_empty() { vec![Len::Star(1.0)] } else { tracks.to_vec() };
    let mut total = 0.0;
    for (i, t) in tracks.iter().enumerate() {
        total += match t {
            Len::Star(_) => cells
                .iter()
                .filter(|(r, c, _)| (if cols { *c } else { *r }).min(tracks.len() - 1) == i)
                .map(|(_, _, e)| if cols { e.desired.w } else { e.desired.h })
                .fold(0.0, f64::max),
            _ => sizes[i],
        };
    }
    total
}

fn prefix(sizes: &[f64], start: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(sizes.len());
    let mut pos = start;
    for s in sizes {
        out.push(pos);
        pos += s;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxy(w: f64, h: f64) -> El {
        El::empty().size(w, h)
    }

    #[test]
    fn margins_and_alignment() {
        let mut root = El::boxed(vec![
            boxy(20.0, 10.0).align(Align::Start, Align::Center).margin(10.0, 0.0, 0.0, 0.0),
            boxy(20.0, 10.0).align(Align::End, Align::Center).margin(0.0, 0.0, 12.0, 0.0),
        ]);
        root.layout(Rect::new(0.0, 0.0, 230.0, 37.0));
        let kids = root.children();
        assert_eq!(kids[0].rect, Rect::new(10.0, 13.5, 20.0, 10.0));
        assert_eq!(kids[1].rect, Rect::new(198.0, 13.5, 20.0, 10.0));
    }

    #[test]
    fn bigger_content_is_centered_in_a_smaller_slot() {
        // A layer laid out at its final size inside a pill that is still small (the island growing).
        let mut root = El::boxed(vec![boxy(371.0, 84.0).align(Align::Center, Align::Start)]);
        root.layout(Rect::new(0.0, 0.0, 126.0, 37.0));
        assert_eq!(root.children()[0].rect, Rect::new((126.0 - 371.0) / 2.0, 0.0, 371.0, 84.0));
    }

    #[test]
    fn stacks_and_star_columns() {
        let mut grid = El::grid(
            vec![Len::Auto, Len::Star(1.0), Len::Auto],
            vec![],
            vec![(0, 0, boxy(52.0, 52.0)), (0, 1, El::empty()), (0, 2, boxy(36.0, 36.0))],
        );
        grid.layout(Rect::new(16.0, 16.0, 339.0, 52.0));
        let kids = grid.children();
        assert_eq!(kids[0].rect.x, 16.0);
        assert_eq!(kids[1].rect, Rect::new(68.0, 16.0, 339.0 - 52.0 - 36.0, 52.0));
        assert_eq!(kids[2].rect.x, 16.0 + 339.0 - 36.0);

        let mut stack = El::hstack(vec![boxy(48.0, 48.0), boxy(48.0, 48.0).margin(10.0, 0.0, 0.0, 0.0)]);
        stack.layout(Rect::new(0.0, 0.0, 300.0, 68.0));
        assert_eq!(stack.children()[1].rect.x, 58.0);
    }

    #[test]
    fn uniform_columns_share_the_width() {
        let mut row = El::uniform(vec![El::empty().margin(4.0, 0.0, 4.0, 0.0), El::empty().margin(4.0, 0.0, 4.0, 0.0)]);
        row.layout(Rect::new(12.0, 0.0, 347.0, 44.0));
        assert_eq!(row.children()[1].rect.x, 12.0 + 347.0 / 2.0 + 4.0);
    }

    #[test]
    fn keys_are_stable() {
        assert_eq!(key("media.next"), key("media.next"));
        assert_ne!(key("media.next"), key("media.prev"));
    }
}
