// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Face panels' drawings: what an environment's client crate draws in the whole of a face panel's frame, from the
//! payload its server side published with it, in Tomori's own shapes and text, which the projector lays out once for
//! the panel's version and size and keeps, as it keeps a chart's face. A client without the drawing a face panel names
//! shows its summary, its body, as a text panel's; and a face in a wall's slot too far away to read is drawn as its
//! title, as every panel is. See `design/panel-faces.md`.
use std::sync::Arc;

use tomori_protocol::{Face, ListRow, Panel};

use crate::canvas::{Canvas, Kind, Label};
use crate::environment::Environment;
use crate::shapes::Shape;
use crate::ui_text::{CELL_HEIGHT, Rgba};

/// The most shapes a drawing draws: one with more is not drawn, and its panel shows its summary instead.
pub const SHAPES: usize = 1024;

/// The most cells of text a drawing's labels hold together: one with more is not drawn.
pub const CELLS: usize = 1024;

/// The most regions a drawing names, one for each of its panel's items, as many as a list has rows.
pub const REGIONS: usize = 30;

/// The colour a face is drawn on where its drawing says none: the plate every panel's face is drawn on.
pub const PLATE: Rgba = [0.02, 0.035, 0.06, 1.0];

/// The colour a region pointed at is outlined in, and how wide, in font pixels: Tomori's own, the same on every face,
/// drawn over the drawing, so that the participant always sees what a press would choose.
const OUTLINE: (Rgba, f32) = (crate::projection::HIGHLIGHT, 1.5);

/// What draws a face panel's face: registered by an environment's client crate for its drawing's key, beside its
/// scenery, with `tomori-render`'s `Sceneries::set_face`. It is asked only when its panel's version or size changes,
/// never each frame, and must not panic: one that does is never asked again while the client runs, and its faces show
/// their summaries.
pub trait Drawer: Send + Sync {
    /// What the face shows: shapes and labels in font pixels from the top-left corner of its frame, `input.size` across
    /// and down, and the regions its items are pointed at by.
    fn draw(&self, input: &Input<'_>) -> Drawing;
}

impl std::fmt::Debug for dyn Drawer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Drawer")
    }
}

/// The drawings a build registers, each for an environment and a key, as `Sceneries::faces` gives them.
pub type Drawers = Vec<(Environment, &'static str, Arc<dyn Drawer>)>;

/// What a drawer is given: the panel's title, its payload, its items, and the size of its whole frame in font pixels,
/// whatever the panel's size, in a wall's slot of any shape or floating.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Input<'a> {
    pub title: &'a str,
    pub data: &'a serde_json::Value,
    pub items: &'a [ListRow],
    /// Its frame's width and height, in font pixels.
    pub size: (f32, f32),
}

impl<'a> Input<'a> {
    /// What `panel`, of `face`, gives a drawer drawing it `size` font pixels across and down.
    pub fn new(panel: &'a Panel, face: &'a Face, size: (f32, f32)) -> Self {
        Self {
            title: &panel.title,
            data: &face.data,
            items: &face.items,
            size,
        }
    }
}

/// What a drawer draws, in font pixels from the top-left corner of its face's frame: what lies wholly within the frame
/// is drawn, and anything else left out.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Drawing {
    /// What it is drawn on, made opaque, as every face is: [`PLATE`] where it says none.
    pub background: Option<Rgba>,
    /// Back to front, under its labels.
    pub shapes: Vec<Shape>,
    pub labels: Vec<Label>,
    /// Where each of its items is pointed at, at most one for each.
    pub regions: Vec<Region>,
}

impl Drawing {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Where one of a face panel's items is pointed at and chosen: a rectangle in font pixels from the top-left corner of
/// its frame, and the item's id.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub item: String,
}

impl Region {
    pub fn new(x: f32, y: f32, (width, height): (f32, f32), item: impl Into<String>) -> Self {
        Self {
            x,
            y,
            width,
            height,
            item: item.into(),
        }
    }

    fn contains(&self, (x, y): (f32, f32)) -> bool {
        (self.x..self.x + self.width).contains(&x) && (self.y..self.y + self.height).contains(&y)
    }
}

/// A drawing as its face shows it: its canvas, unmarked, and each region with the place of its item among the panel's
/// rows, so that pointing at it names that row as pointing at a list's line names its row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Drawn {
    pub(crate) canvas: Canvas,
    pub(crate) regions: Vec<(Region, usize)>,
}

impl Drawn {
    /// Its canvas framed in `border`, the region of the row `hovered` outlined, and the one just `chosen` filled
    /// faintly, as a list marks its rows.
    pub(crate) fn marked(
        &self,
        hovered: Option<usize>,
        chosen: Option<usize>,
        border: Rgba,
    ) -> Canvas {
        let mut canvas = self.canvas.clone();
        canvas.border = Some(border);
        let (left, top) = (canvas.padding.0 as f32, canvas.padding.1 as f32);
        for (region, row) in &self.regions {
            let (x, y) = (region.x - left, region.y - top);
            if chosen == Some(*row) {
                let [r, g, b, _] = OUTLINE.0;
                canvas.shapes.push(Shape::Rect {
                    x,
                    y,
                    width: region.width,
                    height: region.height,
                    color: [r, g, b, 0.18],
                    radius: 2.0,
                    outline: None,
                });
            }
            if hovered == Some(*row) {
                canvas.shapes.push(Shape::Rect {
                    x,
                    y,
                    width: region.width,
                    height: region.height,
                    color: OUTLINE.0,
                    radius: 0.0,
                    outline: Some(OUTLINE.1),
                });
            }
        }
        canvas
    }

    /// The place among its panel's rows of the item whose region holds `point`, in font pixels from the top-left corner
    /// of its frame: the first region that holds it.
    pub(crate) fn item_at(&self, point: (f32, f32)) -> Option<usize> {
        self.regions
            .iter()
            .find(|(region, _)| region.contains(point))
            .map(|(_, row)| *row)
    }
}

/// `panel`'s face, of `face`, as `drawer` draws it on a canvas of `columns` and `rows` of cells at `pixel` metres a font
/// pixel, its padding `padding`, the size every panel of that size is: the drawing placed from the top-left corner of
/// the frame, what lies outside it left out; or none, where the drawer panicked or drew past [`SHAPES`], [`CELLS`] or
/// [`REGIONS`], so that the panel shows its summary instead.
pub(crate) fn draw(
    panel: &Panel,
    face: &Face,
    drawer: &dyn Drawer,
    (columns, rows): (usize, usize),
    (pixel, padding): (f32, (usize, usize)),
) -> Option<Drawn> {
    let mut canvas = Canvas::new(columns, pixel);
    canvas.kind = Kind::PanelFace;
    canvas.padding = padding;
    canvas.rows = vec![crate::ui_text::Row::new(Vec::new()); rows];
    let (width, height) = canvas.size_in_pixels();
    let size = (width as f32, height as f32);
    let input = Input::new(panel, face, size);
    let drawing =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drawer.draw(&input))).ok()?;
    let cells: usize = drawing.labels.iter().map(|label| label.cells.len()).sum();
    if drawing.shapes.len() > SHAPES || cells > CELLS || drawing.regions.len() > REGIONS {
        return None;
    }
    let inside = |bounds: Option<[f32; 4]>| {
        bounds.is_some_and(|[left, top, right, bottom]| {
            left >= 0.0 && top >= 0.0 && right <= size.0 && bottom <= size.1
        })
    };
    // A rectangle, all four of its numbers checked, its sizes never negative.
    let rectangle = |x: f32, y: f32, width: f32, height: f32| {
        ([x, y, width, height].iter().all(|value| value.is_finite())
            && width >= 0.0
            && height >= 0.0)
            .then_some([x, y, x + width, y + height])
    };
    let mut background = drawing.background.unwrap_or(PLATE);
    background[3] = 1.0;
    canvas.background = background;
    // Shapes and labels are placed from inside the padding, the drawing from the frame's corner.
    let (left, top) = (padding.0 as f32, padding.1 as f32);
    canvas.shapes = drawing
        .shapes
        .into_iter()
        .filter(|shape| inside(bounds(shape)))
        .map(|shape| moved(shape, -left, -top))
        .collect();
    canvas.labels = drawing
        .labels
        .into_iter()
        .filter(|label| {
            let height = CELL_HEIGHT as f32 * label.scale;
            inside(rectangle(label.x, label.y, label.width(), height))
        })
        .map(|mut label| {
            label.x -= left;
            label.y -= top;
            label
        })
        .collect();
    let mut regions: Vec<(Region, usize)> = Vec::new();
    for region in drawing.regions {
        let Some(row) = face.items.iter().position(|row| row.id == region.item) else {
            continue;
        };
        if regions.iter().any(|(_, kept)| *kept == row)
            || !inside(rectangle(region.x, region.y, region.width, region.height))
        {
            continue;
        }
        regions.push((region, row));
    }
    Some(Drawn { canvas, regions })
}

/// The box `shape` covers, in font pixels: its left, top, right and bottom edges, before the margin its antialiased
/// edge takes; or none where any of its places or sizes is not a number, or any of its sizes is negative, so that it is
/// left out.
fn bounds(shape: &Shape) -> Option<[f32; 4]> {
    // Every place it is drawn at, and every size it has, checked alike, so that nothing that is not a number, which a
    // comparison or `min` would pass over, reaches the mesh.
    let checked = |points: &[[f32; 2]], sizes: &[f32]| {
        (points.iter().flatten().all(|value| value.is_finite())
            && sizes.iter().all(|size| size.is_finite() && *size >= 0.0))
        .then_some(())
    };
    let span = |points: &[[f32; 2]], reach: f32| {
        let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [x, y] in points {
            left = left.min(*x);
            top = top.min(*y);
            right = right.max(*x);
            bottom = bottom.max(*y);
        }
        [left - reach, top - reach, right + reach, bottom + reach]
    };
    match *shape {
        Shape::Rule {
            from, to, width, ..
        }
        | Shape::Stroke {
            from, to, width, ..
        } => {
            checked(&[from, to], &[width])?;
            Some(span(&[from, to], width / 2.0))
        }
        Shape::Rect {
            x,
            y,
            width,
            height,
            radius,
            outline,
            ..
        } => {
            checked(&[[x, y]], &[width, height, radius, outline.unwrap_or(0.0)])?;
            Some([x, y, x + width, y + height])
        }
        Shape::Bar {
            x,
            y,
            width,
            height,
            from,
            to,
            ..
        } => {
            checked(&[[x, y], [from, to]], &[width, height])?;
            Some([x, y, x + width, y + height])
        }
        Shape::Fill {
            x,
            y,
            width,
            height,
            ..
        } => {
            checked(&[[x, y]], &[width, height])?;
            Some([x, y, x + width, y + height])
        }
        Shape::Check { x, y, size, .. } => {
            checked(&[[x, y]], &[size])?;
            Some([x, y, x + size, y + size])
        }
        Shape::Disc { centre, radius, .. }
        | Shape::Ring { centre, radius, .. }
        | Shape::Dot { centre, radius, .. } => {
            checked(&[centre], &[radius])?;
            Some(span(&[centre], radius))
        }
        Shape::Quad { corners, .. } => {
            checked(&corners, &[])?;
            Some(span(&corners, 0.0))
        }
    }
}

/// `shape` moved `dx` across and `dy` down.
fn moved(shape: Shape, dx: f32, dy: f32) -> Shape {
    let at = |[x, y]: [f32; 2]| [x + dx, y + dy];
    match shape {
        Shape::Rule {
            from,
            to,
            width,
            color,
        } => Shape::Rule {
            from: at(from),
            to: at(to),
            width,
            color,
        },
        Shape::Stroke {
            from,
            to,
            width,
            color,
        } => Shape::Stroke {
            from: at(from),
            to: at(to),
            width,
            color,
        },
        Shape::Rect {
            x,
            y,
            width,
            height,
            color,
            radius,
            outline,
        } => Shape::Rect {
            x: x + dx,
            y: y + dy,
            width,
            height,
            color,
            radius,
            outline,
        },
        Shape::Bar {
            x,
            y,
            width,
            height,
            track,
            fill,
            from,
            to,
        } => Shape::Bar {
            x: x + dx,
            y: y + dy,
            width,
            height,
            track,
            fill,
            from,
            to,
        },
        Shape::Fill {
            x,
            y,
            width,
            height,
            color,
        } => Shape::Fill {
            x: x + dx,
            y: y + dy,
            width,
            height,
            color,
        },
        Shape::Check {
            x,
            y,
            size,
            ticked,
            color,
        } => Shape::Check {
            x: x + dx,
            y: y + dy,
            size,
            ticked,
            color,
        },
        Shape::Disc {
            centre,
            radius,
            color,
        } => Shape::Disc {
            centre: at(centre),
            radius,
            color,
        },
        Shape::Ring {
            centre,
            radius,
            color,
        } => Shape::Ring {
            centre: at(centre),
            radius,
            color,
        },
        Shape::Dot {
            centre,
            radius,
            color,
        } => Shape::Dot {
            centre: at(centre),
            radius,
            color,
        },
        Shape::Quad { corners, color } => Shape::Quad {
            corners: corners.map(at),
            color,
        },
    }
}

#[cfg(test)]
mod tests;

/// Drawings for tests and previews: a 24-hour world clock, as an environment might draw one, its dial lit by day and
/// dark by night, its time large, its area's traffic and a trend under it.
pub mod fixtures {
    use super::{Drawer, Drawing, Input, Region};
    use crate::canvas::Label;
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Rgba, Span, Style, cut};

    const DAY: Rgba = [0.95, 0.66, 0.24, 1.0];
    const TWILIGHT: Rgba = [0.52, 0.26, 0.42, 1.0];
    const NIGHT: Rgba = [0.06, 0.1, 0.26, 1.0];
    const NEEDLE: Rgba = [0.95, 0.97, 1.0, 1.0];
    const RIM: Rgba = [0.1, 0.45, 0.5, 1.0];
    const TEXT: Rgba = [0.85, 0.9, 0.95, 1.0];
    const DIM: Rgba = [0.55, 0.62, 0.7, 1.0];
    const TREND: Rgba = [0.25, 0.42, 0.9, 1.0];

    /// A world clock, drawn from a payload of its `area`, its `city`, its local `hours` into the day, its `time` as
    /// written, its `day`, 24 words of `day`, `twilight` or `night`, its `traffic` as written and its `trend`, numbers;
    /// its region named `dial` where its panel has an item of that id.
    pub struct Clock;

    /// The payload of Europe's clock at 13:20 in Paris in November.
    pub fn europe() -> serde_json::Value {
        let day: Vec<&str> = (0..24)
            .map(|hour| match hour {
                8..=16 => "day",
                7 | 17 => "twilight",
                _ => "night",
            })
            .collect();
        serde_json::json!({
            "area": "Europe",
            "city": "Paris",
            "hours": 13.33,
            "time": "13:20",
            "day": day,
            "traffic": "2.6k / min",
            "trend": [12, 14, 13, 15, 18, 17, 19, 22, 21, 24, 26, 25, 27, 29, 28, 30],
        })
    }

    /// A label of `text` in `colour`, `scale` times as large, centred across on `x`, its top at `y`.
    fn centred(text: &str, colour: Rgba, (x, y): (f32, f32), scale: f32) -> Label {
        let cells = cut(&[Span::new(text, Style::plain(colour))], usize::MAX);
        let width = (cells.len() * CELL_WIDTH) as f32 * scale;
        Label::new(x - width / 2.0, y, cells).scaled(scale)
    }

    /// How far apart large figures stand, of their cells' width: a cell holds a glyph of any width, so that figures
    /// scaled up from it stand as far apart as the cells, wide for figures read together; set closer, as a clock's are.
    const TRACKING: f32 = 0.68;

    /// How far apart large capitals stand, of their cells' width: wider than figures, as bold capitals are.
    const CAPITALS: f32 = 0.86;

    /// `text` in `style`, `scale` times as large, a label for each character `tracking` of a cell apart, from `x`
    /// across, or centred on it, its top at `y`.
    fn figures(
        text: &str,
        style: Style,
        (x, y): (f32, f32),
        (scale, tracking): (f32, f32),
        centre: bool,
    ) -> Vec<Label> {
        let cells = cut(&[Span::new(text, style)], usize::MAX);
        let advance = CELL_WIDTH as f32 * scale * tracking;
        // The last glyph takes its whole cell, the others their advance.
        let width = advance * cells.len().saturating_sub(1) as f32 + CELL_WIDTH as f32 * scale;
        let left = if centre { x - width / 2.0 } else { x };
        cells
            .into_iter()
            .enumerate()
            .map(|(index, cell)| {
                Label::new(left + advance * index as f32, y, vec![cell]).scaled(scale)
            })
            .collect()
    }

    impl Drawer for Clock {
        fn draw(&self, input: &Input<'_>) -> Drawing {
            let data = input.data;
            let text = |key: &str| data[key].as_str().unwrap_or_default().to_owned();
            let (width, frame) = input.size;
            let mut drawing = Drawing::new();
            let line = CELL_HEIGHT as f32;
            // Its title stands in a footer of its own, which nothing else is drawn in: the rest is laid out above it.
            let footer = line * 1.75;
            let height = frame - footer;
            // The dial on the left of a wide face, or at the top of a tall one, the words beside or under it.
            let wide = width > height * 1.2;
            let side = if wide { width * 0.5 } else { width };
            let room = if wide { height } else { height - 4.0 * line };
            let radius = (side.min(room) / 2.0 - 2.5 * line).max(line);
            let centre = if wide {
                [side / 2.0, height / 2.0]
            } else {
                [width / 2.0, room / 2.0 + line]
            };
            let ring = radius * 0.16;
            let at = |hours: f32, reach: f32| {
                let angle = (hours - 12.0) / 24.0 * std::f32::consts::TAU;
                [
                    centre[0] + angle.sin() * reach,
                    centre[1] - angle.cos() * reach,
                ]
            };
            let day = data["day"].as_array().cloned().unwrap_or_default();
            for hour in 0..24 {
                let colour = match day.get(hour).and_then(|word| word.as_str()) {
                    Some("day") => DAY,
                    Some("twilight") => TWILIGHT,
                    _ => NIGHT,
                };
                let (from, to) = (hour as f32 + 0.05, hour as f32 + 0.95);
                drawing.shapes.push(Shape::Quad {
                    corners: [
                        at(from, radius + ring / 2.0),
                        at(to, radius + ring / 2.0),
                        at(from, radius - ring / 2.0),
                        at(to, radius - ring / 2.0),
                    ],
                    color: colour,
                });
            }
            for hour in [0.0, 6.0, 12.0, 18.0] {
                drawing.shapes.push(Shape::Rule {
                    from: at(hour, radius + ring * 0.7),
                    to: at(hour, radius + ring * 1.3),
                    width: 1.5,
                    color: RIM,
                });
            }
            let hours = data["hours"].as_f64().unwrap_or_default() as f32;
            drawing.shapes.push(Shape::Stroke {
                from: at(hours, radius - ring),
                to: at(hours, radius + ring),
                width: 3.0,
                color: NEEDLE,
            });
            // Its time as large as fits inside the ring, up to twice the text's size.
            let time = text("time");
            let across = time.chars().count().max(1) as f32 * CELL_WIDTH as f32 * TRACKING;
            let scale = (radius * 1.4 / across).clamp(1.0, 2.5);
            drawing.labels.extend(figures(
                &time,
                Style::plain(NEEDLE),
                (centre[0], centre[1] - line * 0.4 - line * scale / 2.0),
                (scale, TRACKING),
                true,
            ));
            drawing.labels.push(centred(
                &text("city"),
                DIM,
                (centre[0], centre[1] + line * 1.1),
                1.0,
            ));
            drawing.regions.push(Region::new(
                centre[0] - radius - ring,
                centre[1] - radius - ring,
                (2.0 * (radius + ring), 2.0 * (radius + ring)),
                "dial",
            ));
            // The words: its area large, its traffic, and its trend as bars.
            let (left, top, across) = if wide {
                (side + line * 0.5, line * 1.5, width - side - line * 1.5)
            } else {
                (line, room + line * 0.2, width - 2.0 * line)
            };
            drawing.labels.extend(figures(
                &text("area").to_uppercase(),
                Style::bold(TEXT),
                (left, top),
                (2.0, CAPITALS),
                false,
            ));
            drawing.labels.push(Label::new(
                left,
                top + line * 2.5,
                cut(
                    &[Span::new(text("traffic"), Style::plain(TREND))],
                    usize::MAX,
                ),
            ));
            let trend: Vec<f32> = data["trend"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_f64())
                        .map(|value| value as f32)
                        .collect()
                })
                .unwrap_or_default();
            let high = trend.iter().copied().fold(0.0, f32::max);
            if !trend.is_empty() && high > 0.0 {
                let (base, tall) = (top + line * 6.0, line * 2.2);
                let pitch = across / trend.len() as f32;
                for (index, value) in trend.iter().enumerate() {
                    let up = (value / high * tall).max(1.0);
                    drawing.shapes.push(Shape::Fill {
                        x: left + pitch * index as f32 + pitch * 0.15,
                        y: base - up,
                        width: pitch * 0.7,
                        height: up,
                        color: TREND,
                    });
                }
            }
            // Its title, small, at the foot, so that a face that fills its frame still says what it is.
            drawing.labels.push(Label::new(
                line * 0.5,
                frame - line * 1.25,
                cut(&[Span::new(input.title, Style::plain(DIM))], usize::MAX),
            ));
            drawing
        }
    }
}
