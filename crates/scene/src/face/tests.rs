// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A face panel's drawing is placed from the corner of its frame, kept within it, bounded, and its regions name its
//! items; a drawer that panics or draws past its bounds draws nothing, so that the panel shows its summary.
use super::*;
use crate::ui_text::{Span, Style, cut};
use tomori_protocol::Place;

/// A face panel of the drawing `clock`, with `data` and the items `items`, in a wall's slot.
fn panel(data: serde_json::Value, items: &[&str]) -> Panel {
    Panel::new(
        "panel-1",
        "Europe",
        "World clock Europe (Paris): 13:20 local, day.",
        Place::Wall {
            wall: "wall-1".into(),
            slot: 0,
        },
    )
    .with_face(Some(Face::new("clock", data).with_items(
        items.iter().map(|item| ListRow::new(*item, true)).collect(),
    )))
}

/// A drawer that draws what it is made with.
struct Fixed(Drawing);

impl Drawer for Fixed {
    fn draw(&self, _: &Input<'_>) -> Drawing {
        self.0.clone()
    }
}

/// A drawer that panics.
struct Panics;

impl Drawer for Panics {
    fn draw(&self, _: &Input<'_>) -> Drawing {
        panic!("a drawer that panics")
    }
}

/// `drawing` drawn on a standard slot's canvas of `panel`.
fn drawn(panel: &Panel, drawer: &dyn Drawer) -> Option<Drawn> {
    let face = panel.face.as_deref().unwrap();
    draw(
        panel,
        face,
        drawer,
        (34, 15),
        (0.0019, crate::projection::PADDING),
    )
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Shape {
    Shape::Fill {
        x,
        y,
        width,
        height,
        color: [1.0; 4],
    }
}

fn label(x: f32, y: f32, text: &str) -> Label {
    Label::new(
        x,
        y,
        cut(&[Span::new(text, Style::plain([1.0; 4]))], usize::MAX),
    )
}

#[test]
fn a_drawing_is_placed_from_the_corner_of_the_frame_on_an_opaque_plate() {
    let panel = panel(serde_json::Value::Null, &[]);
    let mut drawing = Drawing::new();
    drawing.shapes.push(rect(0.0, 0.0, 10.0, 10.0));
    drawing.labels.push(label(20.0, 4.0, "13:20").scaled(2.0));
    drawing.background = Some([0.1, 0.2, 0.3, 0.5]);
    let drawn = drawn(&panel, &Fixed(drawing)).unwrap();
    let (left, top) = crate::projection::PADDING;
    assert_eq!(
        drawn.canvas.shapes,
        [rect(-(left as f32), -(top as f32), 10.0, 10.0)]
    );
    assert_eq!(
        (drawn.canvas.labels[0].x, drawn.canvas.labels[0].y),
        (20.0 - left as f32, 4.0 - top as f32)
    );
    assert_eq!(drawn.canvas.labels[0].scale, 2.0);
    assert_eq!(drawn.canvas.background, [0.1, 0.2, 0.3, 1.0]);
    assert_eq!(drawn.canvas.kind, Kind::PanelFace);
    // Its frame is a standard slot's, as every panel's in it is.
    assert_eq!(drawn.canvas.size_in_pixels(), (34 * 9 + 12, 15 * 12 + 10));
    // Without a colour of its own, it is drawn on the plate.
    let plain = self::drawn(&panel, &Fixed(Drawing::new())).unwrap();
    assert_eq!(plain.canvas.background, PLATE);
}

#[test]
fn what_lies_outside_the_frame_is_left_out_whole() {
    let panel = panel(serde_json::Value::Null, &["a"]);
    let (width, height) = (34.0 * 9.0 + 12.0, 15.0 * 12.0 + 10.0);
    let mut drawing = Drawing::new();
    drawing.shapes = vec![
        rect(0.0, 0.0, width, height),
        rect(-1.0, 0.0, 5.0, 5.0),
        rect(width - 4.0, 0.0, 5.0, 5.0),
        rect(0.0, height - 4.0, 5.0, 5.0),
        rect(f32::NAN, 0.0, 5.0, 5.0),
        Shape::Disc {
            centre: [3.0, 3.0],
            radius: 4.0,
            color: [1.0; 4],
        },
        Shape::Stroke {
            from: [10.0, 10.0],
            to: [20.0, 20.0],
            width: 2.0,
            color: [1.0; 4],
        },
    ];
    drawing.labels = vec![
        label(0.0, 0.0, "fits"),
        label(width - 10.0, 0.0, "too wide"),
        label(0.0, height - 30.0, "too tall").scaled(4.0),
    ];
    drawing.regions = vec![
        Region::new(-1.0, 0.0, (10.0, 10.0), "a"),
        Region::new(0.0, 0.0, (10.0, 10.0), "a"),
    ];
    let drawn = drawn(&panel, &Fixed(drawing)).unwrap();
    assert_eq!(drawn.canvas.shapes.len(), 2);
    assert_eq!(drawn.canvas.labels.len(), 1);
    assert_eq!(drawn.regions.len(), 1);
}

#[test]
fn a_drawing_past_its_bounds_or_a_drawer_that_panics_draws_nothing() {
    let panel = panel(serde_json::Value::Null, &[]);
    let mut many = Drawing::new();
    many.shapes = vec![rect(0.0, 0.0, 1.0, 1.0); SHAPES + 1];
    assert!(drawn(&panel, &Fixed(many)).is_none());
    let mut wordy = Drawing::new();
    wordy.labels = vec![label(0.0, 0.0, &"x".repeat(CELLS / 2 + 1)); 2];
    assert!(drawn(&panel, &Fixed(wordy)).is_none());
    let mut named = Drawing::new();
    named.regions = vec![Region::new(0.0, 0.0, (1.0, 1.0), "a"); REGIONS + 1];
    assert!(drawn(&panel, &Fixed(named)).is_none());
    assert!(drawn(&panel, &Panics).is_none());
    let mut most = Drawing::new();
    most.shapes = vec![rect(0.0, 0.0, 1.0, 1.0); SHAPES];
    assert!(drawn(&panel, &Fixed(most)).is_some());
}

#[test]
fn regions_name_the_panel_s_items_by_their_place_among_its_rows() {
    let panel = panel(serde_json::Value::Null, &["first", "second"]);
    let mut drawing = Drawing::new();
    drawing.regions = vec![
        Region::new(10.0, 10.0, (20.0, 20.0), "second"),
        Region::new(15.0, 15.0, (20.0, 20.0), "first"),
        Region::new(50.0, 10.0, (20.0, 20.0), "gone"),
        Region::new(80.0, 10.0, (20.0, 20.0), "second"),
    ];
    let drawn = drawn(&panel, &Fixed(drawing)).unwrap();
    assert_eq!(drawn.regions.len(), 2);
    // The first region that holds a point names its item.
    assert_eq!(drawn.item_at((12.0, 12.0)), Some(1));
    assert_eq!(drawn.item_at((20.0, 20.0)), Some(1));
    assert_eq!(drawn.item_at((33.0, 33.0)), Some(0));
    assert_eq!(drawn.item_at((55.0, 15.0)), None);
    assert_eq!(drawn.item_at((85.0, 15.0)), None);
    // The region of the row pointed at is outlined, and the one just chosen filled faintly, framed as asked.
    let border = [0.5; 4];
    assert_eq!(drawn.marked(None, None, border).shapes, drawn.canvas.shapes);
    let marked = drawn.marked(Some(1), Some(0), border);
    assert_eq!(marked.border, Some(border));
    let (left, top) = crate::projection::PADDING;
    assert!(marked.shapes.iter().any(|shape| matches!(
        shape,
        Shape::Rect { x, outline: Some(_), .. } if *x == 10.0 - left as f32
    )));
    assert!(marked.shapes.iter().any(|shape| matches!(
        shape,
        Shape::Rect { y, outline: None, .. } if *y == 15.0 - top as f32
    )));
}

#[test]
fn the_world_clock_draws_its_dial_words_and_trend_within_its_frame_at_any_size() {
    let panel = panel(fixtures::europe(), &["dial"]);
    let face = panel.face.as_deref().unwrap();
    for cells in [(34, 15), (48, 17), (20, 30)] {
        let drawn = draw(
            &panel,
            face,
            &fixtures::Clock,
            cells,
            (0.0019, crate::projection::PADDING),
        )
        .unwrap();
        let quads = drawn
            .canvas
            .shapes
            .iter()
            .filter(|shape| matches!(shape, Shape::Quad { .. }))
            .count();
        assert_eq!(quads, 24, "{cells:?}: an arc for every hour");
        assert!(
            drawn
                .canvas
                .labels
                .iter()
                .filter(|label| label.scale > 1.0)
                .map(plain)
                .collect::<String>()
                .contains("13:20"),
            "{cells:?}: its time, large"
        );
        assert_eq!(drawn.regions.len(), 1, "{cells:?}: its dial is pointed at");
        // Nothing but its title is drawn in its footer, the last row and three quarters of the frame.
        let footer =
            drawn.canvas.size_in_pixels().1 as f32 - 1.75 * crate::ui_text::CELL_HEIGHT as f32;
        let top = crate::projection::PADDING.1 as f32;
        for shape in &drawn.canvas.shapes {
            assert!(
                bounds(shape).unwrap()[3] + top <= footer,
                "{cells:?}: {shape:?}"
            );
        }
        let words = drawn
            .canvas
            .labels
            .iter()
            .filter(|label| plain(label) != "Europe");
        for label in words {
            let bottom = label.y + top + crate::ui_text::CELL_HEIGHT as f32 * label.scale;
            assert!(bottom <= footer, "{cells:?}: {}", plain(label));
        }
    }
    // A payload of nothing draws its ring as night, and nothing more of what it does not say.
    let empty = self::panel(serde_json::Value::Null, &[]);
    let drawn = drawn(&empty, &fixtures::Clock).unwrap();
    assert!(drawn.regions.is_empty());
}

fn plain(label: &Label) -> String {
    label.cells.iter().map(|(ch, _)| ch).collect()
}

#[test]
fn a_drawer_is_debugged_as_a_drawer() {
    let drawer: Arc<dyn Drawer> = Arc::new(fixtures::Clock);
    assert_eq!(format!("{drawer:?}"), "Drawer");
}

#[test]
fn every_shape_within_the_frame_is_moved_from_its_corner_to_the_canvas_s_padding() {
    let panel = panel(serde_json::Value::Null, &[]);
    let colour = [1.0; 4];
    let shapes = vec![
        Shape::Rule {
            from: [10.0, 10.0],
            to: [20.0, 10.0],
            width: 1.0,
            color: colour,
        },
        Shape::Stroke {
            from: [10.0, 12.0],
            to: [20.0, 12.0],
            width: 1.0,
            color: colour,
        },
        Shape::Rect {
            x: 10.0,
            y: 14.0,
            width: 4.0,
            height: 4.0,
            color: colour,
            radius: 1.0,
            outline: None,
        },
        Shape::Bar {
            x: 10.0,
            y: 20.0,
            width: 20.0,
            height: 2.0,
            track: colour,
            fill: colour,
            from: 0.0,
            to: 0.5,
        },
        rect(10.0, 24.0, 4.0, 4.0),
        Shape::Check {
            x: 10.0,
            y: 30.0,
            size: 6.0,
            ticked: true,
            color: colour,
        },
        Shape::Disc {
            centre: [40.0, 40.0],
            radius: 3.0,
            color: colour,
        },
        Shape::Ring {
            centre: [50.0, 40.0],
            radius: 3.0,
            color: colour,
        },
        Shape::Dot {
            centre: [60.0, 40.0],
            radius: 1.0,
            color: colour,
        },
        Shape::Quad {
            corners: [[70.0, 40.0], [80.0, 40.0], [70.0, 50.0], [80.0, 50.0]],
            color: colour,
        },
    ];
    let mut drawing = Drawing::new();
    drawing.shapes = shapes.clone();
    let drawn = drawn(&panel, &Fixed(drawing)).unwrap();
    let (left, top) = crate::projection::PADDING;
    let (dx, dy) = (left as f32, top as f32);
    assert_eq!(drawn.canvas.shapes.len(), shapes.len());
    for (moved, shape) in drawn.canvas.shapes.iter().zip(&shapes) {
        // Moved back by the padding, it is the shape drawn, and its box moves with it.
        assert_eq!(super::moved(*moved, dx, dy), *shape);
        let (before, after) = (bounds(shape).unwrap(), bounds(moved).unwrap());
        assert_eq!(
            [after[0] + dx, after[1] + dy],
            [before[0], before[1]],
            "{shape:?}"
        );
    }
}

#[test]
fn a_shape_label_or_region_with_a_place_that_is_not_a_number_or_a_negative_size_is_left_out() {
    let panel = panel(serde_json::Value::Null, &["a"]);
    let colour = [1.0; 4];
    let nan = f32::NAN;
    let shapes = [
        Shape::Rule {
            from: [nan, 10.0],
            to: [20.0, 10.0],
            width: 1.0,
            color: colour,
        },
        Shape::Rule {
            from: [10.0, 10.0],
            to: [20.0, 10.0],
            width: -1.0,
            color: colour,
        },
        Shape::Stroke {
            from: [10.0, 10.0],
            to: [20.0, nan],
            width: 1.0,
            color: colour,
        },
        Shape::Rect {
            x: 10.0,
            y: 10.0,
            width: -4.0,
            height: 4.0,
            color: colour,
            radius: 0.0,
            outline: None,
        },
        Shape::Rect {
            x: 10.0,
            y: 10.0,
            width: 4.0,
            height: 4.0,
            color: colour,
            radius: nan,
            outline: None,
        },
        Shape::Rect {
            x: 10.0,
            y: 10.0,
            width: 4.0,
            height: 4.0,
            color: colour,
            radius: 0.0,
            outline: Some(-1.0),
        },
        Shape::Bar {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: -2.0,
            track: colour,
            fill: colour,
            from: 0.0,
            to: 0.5,
        },
        Shape::Bar {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 2.0,
            track: colour,
            fill: colour,
            from: 0.0,
            to: nan,
        },
        rect(10.0, nan, 4.0, 4.0),
        rect(10.0, 10.0, 4.0, -4.0),
        Shape::Check {
            x: 10.0,
            y: 10.0,
            size: -6.0,
            ticked: true,
            color: colour,
        },
        Shape::Disc {
            centre: [nan, 40.0],
            radius: 3.0,
            color: colour,
        },
        Shape::Ring {
            centre: [50.0, 40.0],
            radius: -3.0,
            color: colour,
        },
        Shape::Dot {
            centre: [60.0, nan],
            radius: 1.0,
            color: colour,
        },
        Shape::Quad {
            corners: [[70.0, 40.0], [nan, 40.0], [70.0, 50.0], [80.0, 50.0]],
            color: colour,
        },
        Shape::Quad {
            corners: [
                [70.0, 40.0],
                [80.0, 40.0],
                [70.0, f32::INFINITY],
                [80.0, 50.0],
            ],
            color: colour,
        },
    ];
    for shape in shapes {
        assert_eq!(bounds(&shape), None, "{shape:?}");
        let mut drawing = Drawing::new();
        drawing.shapes.push(shape);
        assert!(
            drawn(&panel, &Fixed(drawing))
                .unwrap()
                .canvas
                .shapes
                .is_empty(),
            "{shape:?}"
        );
    }
    let mut drawing = Drawing::new();
    drawing.labels = vec![label(nan, 0.0, "x"), label(0.0, f32::INFINITY, "x")];
    drawing.regions = vec![
        Region::new(nan, 0.0, (10.0, 10.0), "a"),
        Region::new(20.0, 0.0, (-10.0, 10.0), "a"),
        Region::new(0.0, 0.0, (10.0, nan), "a"),
    ];
    let drawn = drawn(&panel, &Fixed(drawing)).unwrap();
    assert!(drawn.canvas.labels.is_empty());
    assert!(drawn.regions.is_empty());
}
