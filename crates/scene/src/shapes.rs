// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The shapes every panel, card, menu and sign draws besides its text: lines, boxes, bars, dots, rings and checkboxes,
//! drawn as geometry rather than as glyphs. Text is drawn as text; a rule, a box, a bar or an indicator is drawn as a
//! [`Shape`], so that it stays crisp and as bright all along as a panel is scaled, scrolled, wrapped and seen from far
//! away (see `design/rendering.md`).
//!
//! A shape is placed in font pixels, the units text is laid out in, from a canvas's top-left corner with y down, as a
//! cell's glyph is, so that layout mixes the two. [`Shape::pieces`] turns it into [`Piece`]s, quads drawn from the
//! atlas's shape tiles, [`tomori_text::atlas::SQUARE`], [`tomori_text::atlas::DISC`] and [`tomori_text::atlas::RING`], by the same pipeline and shader as
//! glyphs, in the same mesh, so that a panel's shapes cost no draw of their own, are tested against depth as its text
//! is, and are drawn alike in both eyes. Each piece samples the part of its tile where the field is that of the shape's
//! edge, stretched where the field is flat, so that the shader antialiases the edge as it does a glyph's, by how fast the
//! field changes across the pixel, and, where a pixel spans several of the atlas's, averages samples across it: a thin
//! line far away fades rather than shimmering. Each shape reaches a margin beyond its edge, two font pixels for lines and
//! boxes, for its antialiased edge to fall in, and so that a line thinner than a pixel always covers one.
//!
//! Layout that lays text out in cells marks a cell to be drawn as a shape with a [`Mark`] in its style
//! ([`Style::marked`]): the cell keeps a character that stands in for it, which the console prints, and a canvas draws
//! the shape in its place, a run of a line's cells along a row or down a column as one line.
use tomori_text::atlas::{DISC, FRAME, FRAME_WIDTH, Figure, RING, SOLID, SPREAD, SQUARE, WEDGE};

use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Rgba, Row, Span, Style};

/// The atlas's pixels to a font pixel for lines and boxes, so that the field's spread is a font pixel and a line a font
/// pixel wide is full at its middle.
const SCALE: f32 = SPREAD;
/// How far beyond its edge a shape's pieces reach, in the atlas's pixels: past the field's spread into the empty field
/// around its tile, so that a line thinner than a screen pixel, far away, always covers the middle of a pixel or two,
/// whose samples find it, wherever it falls, rather than now one and now none, which would shimmer.
const REACH: f32 = 2.0 * SPREAD;
/// A rounded corner or a dot smaller than this, in font pixels, is drawn square, as its margin would be too small for its
/// edge.
const SMALLEST_RADIUS: f32 = 1.0;

/// A shape, in font pixels from a canvas's top-left corner, y down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A straight line from `from` to `to`, `width` across, with square ends.
    Rule {
        from: [f32; 2],
        to: [f32; 2],
        width: f32,
        color: Rgba,
    },
    /// A box from `x`, `y`, `width` across and `height` down: filled, its corners rounded by `radius`, or, with an
    /// `outline`, a frame that wide inside its edge, its corners square.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Rgba,
        radius: f32,
        outline: Option<f32>,
    },
    /// A track along the box from `x`, `y`, `width` across and `height` down, along its longer side, filled from `from`
    /// to `to` of its length, from 0 to 1: a progress bar from 0, or a scroll bar's thumb where its view is.
    Bar {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        track: Rgba,
        fill: Rgba,
        from: f32,
        to: f32,
    },
    /// A box from `x`, `y`, `width` across and `height` down, filled to its very edges, which are not antialiased, so
    /// that it meets a neighbour without a seam or lies wholly under what covers it: a background, or where an image
    /// will be.
    Fill {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Rgba,
    },
    /// A filled circle.
    Disc {
        centre: [f32; 2],
        radius: f32,
        color: Rgba,
    },
    /// A circle's outline, a third of its radius wide, inside it, as [`tomori_text::atlas::RING`] is.
    Ring {
        centre: [f32; 2],
        radius: f32,
        color: Rgba,
    },
    /// A checkbox, `size` a side from `x`, `y`: a square frame, or, where `ticked`, a filled square with a dark tick in
    /// it, as GitHub shows one.
    Check {
        x: f32,
        y: f32,
        size: f32,
        ticked: bool,
        color: Rgba,
    },
}

/// A quad of a shape, drawn from the atlas: its corners, top-left, top-right, bottom-left and bottom-right, in font
/// pixels from the canvas's top-left corner, y down, and where in the atlas each is, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub corners: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub color: Rgba,
}

impl Piece {
    /// Its two triangles, as six corners, each with where in the atlas it is: top-left, top-right, bottom-left, then
    /// top-right, bottom-right, bottom-left.
    pub fn triangles(&self) -> [([f32; 2], [f32; 2]); 6] {
        [0, 1, 2, 1, 3, 2].map(|corner| (self.corners[corner], self.uv[corner]))
    }

    /// It cut across into strips no wider than `widest` font pixels along its top edge, each drawing its part, so that a
    /// surface that bends, as a worker's sign bows, bends it too.
    pub fn strips(&self, widest: f32) -> Vec<Self> {
        let [a, b, c, d] = self.corners;
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let count = (length / widest).ceil().max(1.0) as usize;
        let mix = |from: [f32; 2], to: [f32; 2], t: f32| {
            [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ]
        };
        (0..count)
            .map(|index| {
                let (from, to) = (
                    index as f32 / count as f32,
                    (index + 1) as f32 / count as f32,
                );
                let [ua, ub, uc, ud] = self.uv;
                Self {
                    corners: [
                        mix(a, b, from),
                        mix(a, b, to),
                        mix(c, d, from),
                        mix(c, d, to),
                    ],
                    uv: [
                        mix(ua, ub, from),
                        mix(ua, ub, to),
                        mix(uc, ud, from),
                        mix(uc, ud, to),
                    ],
                    color: self.color,
                }
            })
            .collect()
    }
}

impl Shape {
    /// A box, filled, with square corners.
    pub const fn rect(x: f32, y: f32, width: f32, height: f32, color: Rgba) -> Self {
        Self::Rect {
            x,
            y,
            width,
            height,
            color,
            radius: 0.0,
            outline: None,
        }
    }

    /// A box's frame, `width` wide inside its edge.
    pub const fn frame(x: f32, y: f32, size: [f32; 2], width: f32, color: Rgba) -> Self {
        Self::Rect {
            x,
            y,
            width: size[0],
            height: size[1],
            color,
            radius: 0.0,
            outline: Some(width),
        }
    }

    /// The pieces that draw it, added to `pieces`; none for a shape that is wholly transparent or has no size.
    pub fn pieces(&self, pieces: &mut Vec<Piece>) {
        match *self {
            Self::Rule {
                from,
                to,
                width,
                color,
            } => {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                let length = dx.hypot(dy);
                if length <= 0.0 {
                    return;
                }
                let along = [dx / length, dy / length];
                let place = Place {
                    middle: [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0],
                    along,
                    across: [-along[1], along[0]],
                };
                place.fill(pieces, [length / 2.0, width / 2.0], 0.0, color);
            }
            Self::Rect {
                x,
                y,
                width,
                height,
                color,
                radius,
                outline,
            } => match outline {
                None => Place::of(x, y, width, height).fill(
                    pieces,
                    [width / 2.0, height / 2.0],
                    radius,
                    color,
                ),
                // Drawn from the frame's tile, its corners from the tile's and its sides from the middles of the tile's,
                // stretched, so that its field is continuous across its width and it is antialiased inside and out.
                Some(thick) => {
                    let thick = thick.min(width / 2.0).min(height / 2.0);
                    if thick <= 0.0 {
                        return;
                    }
                    let scale = FRAME_WIDTH / thick;
                    let place = Place::of(x, y, width, height);
                    // Each corner and side reaches as far in as its band and the margin inside it, beyond which the
                    // tile is hollow.
                    let depth = thick + margin(scale);
                    let across = slices(width / 2.0, FRAME.reach, scale, depth);
                    let down = slices(height / 2.0, FRAME.reach, scale, depth);
                    for &(x, u) in &across {
                        for &(y, v) in &down {
                            // Its middle is hollow.
                            if u != [0.0, 0.0] || v != [0.0, 0.0] {
                                place.piece(pieces, x, y, FRAME, u, v, color);
                            }
                        }
                    }
                }
            },
            Self::Bar {
                x,
                y,
                width,
                height,
                track,
                fill,
                from,
                to,
            } => {
                Self::rect(x, y, width, height, track).pieces(pieces);
                let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
                let filled = if width >= height {
                    Self::rect(x + width * from, y, width * (to - from), height, fill)
                } else {
                    Self::rect(x, y + height * from, width, height * (to - from), fill)
                };
                filled.pieces(pieces);
            }
            Self::Fill {
                x,
                y,
                width,
                height,
                color,
            } => {
                if width > 0.0 && height > 0.0 && color[3] > 0.0 {
                    pieces.push(Piece {
                        corners: [
                            [x, y],
                            [x + width, y],
                            [x, y + height],
                            [x + width, y + height],
                        ],
                        uv: [SOLID; 4],
                        color,
                    });
                }
            }
            Self::Disc {
                centre,
                radius,
                color,
            } => Place::of(
                centre[0] - radius,
                centre[1] - radius,
                2.0 * radius,
                2.0 * radius,
            )
            .fill(pieces, [radius, radius], radius, color),
            Self::Ring {
                centre,
                radius,
                color,
            } => {
                if radius < SMALLEST_RADIUS {
                    return;
                }
                let place = Place {
                    middle: centre,
                    along: [1.0, 0.0],
                    across: [0.0, 1.0],
                };
                let scale = RING.reach / radius;
                let reach = radius + REACH / scale;
                place.piece(
                    pieces,
                    [-reach, reach],
                    [-reach, reach],
                    RING,
                    [-reach * scale, reach * scale],
                    [-reach * scale, reach * scale],
                    color,
                );
            }
            Self::Check {
                x,
                y,
                size,
                ticked,
                color,
            } => {
                if !ticked {
                    Self::frame(x, y, [size, size], size * CHECK_FRAME, color).pieces(pieces);
                    return;
                }
                Self::rect(x, y, size, size, color).pieces(pieces);
                let at = |[u, v]: [f32; 2]| [x + u * size, y + v * size];
                for [from, to] in TICK {
                    Self::Rule {
                        from: at(from),
                        to: at(to),
                        width: size * CHECK_TICK,
                        color: [TICK_INK[0], TICK_INK[1], TICK_INK[2], color[3]],
                    }
                    .pieces(pieces);
                }
            }
        }
    }
}

/// The colour of a ticked checkbox's tick, as dark as a panel's backing.
const TICK_INK: Rgba = [0.01, 0.02, 0.04, 1.0];
/// How wide a checkbox's frame is, and its tick, as a share of its side.
const CHECK_FRAME: f32 = 0.15;
const CHECK_TICK: f32 = 0.18;
/// A tick's two strokes, from a checkbox's top-left corner, as shares of its side: down to the bottom of the tick, and
/// up from just before it, so that the two overlap where they meet.
const TICK: [[[f32; 2]; 2]; 2] = [[[0.22, 0.52], [0.43, 0.73]], [[0.38, 0.73], [0.8, 0.26]]];

/// Where a box is: its middle, and the directions of its length and its width, of a font pixel each.
#[derive(Clone, Copy, Debug)]
struct Place {
    middle: [f32; 2],
    along: [f32; 2],
    across: [f32; 2],
}

impl Place {
    /// The box from `x`, `y`, `width` across and `height` down, upright.
    fn of(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            middle: [x + width / 2.0, y + height / 2.0],
            along: [1.0, 0.0],
            across: [0.0, 1.0],
        }
    }

    /// A box `half` its length and width either side of its middle, filled, with corners rounded by `radius`, from the
    /// disc's tile, or square, from the square's.
    fn fill(&self, pieces: &mut Vec<Piece>, half: [f32; 2], radius: f32, color: Rgba) {
        if half[0] <= 0.0 || half[1] <= 0.0 || color[3] <= 0.0 {
            return;
        }
        let radius = radius.min(half[0]).min(half[1]);
        let (figure, scale) = if radius >= SMALLEST_RADIUS {
            (DISC, DISC.reach / radius)
        } else {
            (SQUARE, SCALE)
        };
        let depth = figure.reach / scale;
        let across = slices(half[0], figure.reach, scale, depth);
        let down = slices(half[1], figure.reach, scale, depth);
        // A side thinner than the square's edge field reaches would be drawn from the square's two edges, which do not
        // meet: so where the other side is flat, it is drawn across from one column of the wedge.
        let thin = |half: f32| figure == SQUARE && half < figure.reach / scale;
        let (thin_x, thin_y) = (thin(half[0]), thin(half[1]));
        let flat = |span: [f32; 2]| span == [0.0, 0.0];
        for (i, &(x, u)) in across.iter().enumerate() {
            for (j, &(y, v)) in down.iter().enumerate() {
                if thin_y && flat(u) {
                    if j == 0 {
                        let [low, high, column] = band(half[1], scale);
                        let uv = [[column, low], [column, low], [column, high], [column, high]];
                        self.raw(
                            pieces,
                            x,
                            [-half[1] - margin(scale), half[1] + margin(scale)],
                            uv,
                            color,
                        );
                    }
                } else if thin_x && flat(v) {
                    if i == 0 {
                        let [low, high, column] = band(half[0], scale);
                        let uv = [[column, low], [column, high], [column, low], [column, high]];
                        self.raw(
                            pieces,
                            [-half[0] - margin(scale), half[0] + margin(scale)],
                            y,
                            uv,
                            color,
                        );
                    }
                } else {
                    self.piece(pieces, x, y, figure, u, v, color);
                }
            }
        }
    }

    /// The piece from `x[0]` to `x[1]` along the box and `y[0]` to `y[1]` across it, from its middle, drawn from
    /// `figure`'s tile from `u[0]` to `u[1]` and `v[0]` to `v[1]` from its middle.
    #[allow(clippy::too_many_arguments)]
    fn piece(
        &self,
        pieces: &mut Vec<Piece>,
        x: [f32; 2],
        y: [f32; 2],
        figure: Figure,
        u: [f32; 2],
        v: [f32; 2],
        color: Rgba,
    ) {
        let tile = |u: f32, v: f32| [figure.middle[0] + u, figure.middle[1] + v];
        let uv = [
            tile(u[0], v[0]),
            tile(u[1], v[0]),
            tile(u[0], v[1]),
            tile(u[1], v[1]),
        ];
        self.raw(pieces, x, y, uv, color);
    }

    /// The piece from `x[0]` to `x[1]` along the box and `y[0]` to `y[1]` across it, from its middle, its corners,
    /// top-left, top-right, bottom-left and bottom-right, drawn from `uv` in the atlas.
    fn raw(
        &self,
        pieces: &mut Vec<Piece>,
        x: [f32; 2],
        y: [f32; 2],
        uv: [[f32; 2]; 4],
        color: Rgba,
    ) {
        let at = |x: f32, y: f32| {
            [
                self.middle[0] + self.along[0] * x + self.across[0] * y,
                self.middle[1] + self.along[1] * x + self.across[1] * y,
            ]
        };
        pieces.push(Piece {
            corners: [
                at(x[0], y[0]),
                at(x[1], y[0]),
                at(x[0], y[1]),
                at(x[1], y[1]),
            ],
            uv,
            color,
        });
    }
}

/// How far beyond its edge a shape drawn at `scale` reaches, in font pixels.
fn margin(scale: f32) -> f32 {
    REACH / scale
}

/// Where in the atlas a band `half` either side of its middle, at `scale`, is drawn across from, with its margin: from
/// and to, down the wedge, and the wedge's column where it is as wide.
fn band(half: f32, scale: f32) -> [f32; 3] {
    let middle = WEDGE.start[1];
    let reach = half * scale + REACH;
    [
        middle - reach,
        middle + reach,
        WEDGE.column(2.0 * half * scale),
    ]
}

/// The spans along one side of a box `half` either side of its middle, each with the span of its figure's tile it is
/// drawn from, from the tile's middle, whose edge is `reach` from it, at `scale` of the atlas's pixels to a font pixel:
/// near each end, from its margin outside the edge to `depth` in, where the tile's field is flat, or to the middle of a
/// box too thin for both; and between, the tile's middle, stretched.
fn slices(half: f32, reach: f32, scale: f32, depth: f32) -> Vec<([f32; 2], [f32; 2])> {
    let margin = margin(scale);
    let depth = depth.min(half);
    let mut spans = vec![(
        [-half - margin, -half + depth],
        [-reach - REACH, -reach + depth * scale],
    )];
    if depth < half {
        spans.push(([-half + depth, half - depth], [0.0, 0.0]));
    }
    spans.push((
        [half - depth, half + margin],
        [reach - depth * scale, reach + REACH],
    ));
    spans
}

/// A shape drawn in a cell, or in a run of cells, in place of its character, which stands in for it where text is
/// printed, as on the console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A line across the middle of its cells, a rule: a run of them along a row is one line.
    Rule,
    /// A line down its cells, near their left edge, between a table's columns: a run of them down a column is one line.
    Line,
    /// A quote's bar, down the middle of its cells: a run of them down a column is one bar.
    Quote,
    /// A checkbox, empty or ticked.
    Check { ticked: bool },
    /// A dot, such as a status's.
    Dot,
    /// A ring, with a dot in it where `dot`.
    Ring { dot: bool },
}

/// Where in a cell its marks are drawn, in font pixels from its top-left corner: lines across through its middle,
/// lines and bars down, and a dot or ring about the middle of a lowercase letter, as their glyphs were.
const RULE_Y: f32 = 6.0;
const RULE_WIDTH: f32 = 1.0;
const LINE_X: f32 = 0.7;
const QUOTE_WIDTH: f32 = 1.5;
const DOT: [f32; 2] = [4.5, 5.9];
const DOT_RADIUS: f32 = 2.5;
const RING_RADIUS: f32 = 2.6;
/// A checkbox's top-left corner in its cell and its side: as wide as a capital, and standing on the baseline.
const CHECK: [f32; 3] = [0.5, 1.0, 8.0];

impl Mark {
    /// The character that stands in for it.
    pub const fn stand_in(self) -> char {
        match self {
            Self::Rule => '─',
            Self::Line => '▏',
            Self::Quote => '│',
            Self::Check { ticked: false } => '☐',
            Self::Check { ticked: true } => '☑',
            Self::Dot => '●',
            Self::Ring { dot: false } => '○',
            Self::Ring { dot: true } => '◉',
        }
    }

    /// Which way a run of it joins: along a row, down a column, or not at all, a shape in each cell.
    pub(crate) const fn joins(self) -> Join {
        match self {
            Self::Rule => Join::Across,
            Self::Line | Self::Quote => Join::Down,
            _ => Join::Alone,
        }
    }

    /// The shapes that draw it in `cells` cells along a row from `x`, or down a column from `y`, as it joins, in `color`:
    /// `x` and `y` the top-left corner of its first cell, in font pixels.
    pub(crate) fn shapes(self, x: f32, y: f32, cells: usize, color: Rgba) -> Vec<Shape> {
        let (width, height) = (CELL_WIDTH as f32, CELL_HEIGHT as f32);
        let run = cells as f32;
        let down = |at: f32, width: f32| Shape::Rule {
            from: [x + at, y],
            to: [x + at, y + height * run],
            width,
            color,
        };
        let centre = [x + DOT[0], y + DOT[1]];
        match self {
            Self::Rule => vec![Shape::Rule {
                from: [x, y + RULE_Y],
                to: [x + width * run, y + RULE_Y],
                width: RULE_WIDTH,
                color,
            }],
            Self::Line => vec![down(LINE_X, RULE_WIDTH)],
            Self::Quote => vec![down(width / 2.0, QUOTE_WIDTH)],
            Self::Check { ticked } => vec![Shape::Check {
                x: x + CHECK[0],
                y: y + CHECK[1],
                size: CHECK[2],
                ticked,
                color,
            }],
            Self::Dot => vec![Shape::Disc {
                centre,
                radius: DOT_RADIUS,
                color,
            }],
            Self::Ring { dot } => {
                let mut shapes = vec![Shape::Ring {
                    centre,
                    radius: RING_RADIUS,
                    color,
                }];
                if dot {
                    shapes.push(Shape::Disc {
                        centre,
                        radius: RING_RADIUS / 2.0,
                        color,
                    });
                }
                shapes
            }
        }
    }

    /// A row of `cells` cells of it, in `style`, such as a rule across a panel.
    pub fn row(self, cells: usize, style: Style) -> Row {
        Row::new(crate::ui_text::cut(&[self.span(cells, style)], cells))
    }

    /// `cells` cells of it, in `style`, as a span to lay out with text.
    pub fn span(self, cells: usize, style: Style) -> Span {
        Span::new(
            std::iter::repeat_n(self.stand_in(), cells).collect::<String>(),
            style.marked(self),
        )
    }
}

/// Which way a run of a [`Mark`]'s cells joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Join {
    Across,
    Down,
    Alone,
}

#[cfg(test)]
mod tests;
