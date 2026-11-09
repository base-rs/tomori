// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_text::atlas::FRAME;
use tomori_text::atlas::{self, WEDGE, WIDTH};

const WHITE: Rgba = [1.0; 4];
const FAINT: Rgba = [0.6, 0.66, 0.74, 0.45];

fn pieces(shape: Shape) -> Vec<Piece> {
    let mut pieces = Vec::new();
    shape.pieces(&mut pieces);
    pieces
}

/// The box an upright piece covers: left, top, right, bottom.
fn bounds(piece: &Piece) -> [f32; 4] {
    let xs = piece.corners.map(|[x, _]| x);
    let ys = piece.corners.map(|[_, y]| y);
    let least = |values: [f32; 4]| values.into_iter().fold(f32::MAX, f32::min);
    let most = |values: [f32; 4]| values.into_iter().fold(f32::MIN, f32::max);
    [least(xs), least(ys), most(xs), most(ys)]
}

/// The field the shader reads at `point`, from the piece that covers it, as the atlas holds it: where in the piece the
/// point is, as a share of each side, gives where in the tile, read between its pixels as a sampler reads it. `None`
/// where no piece covers it; more than one covering it is an overlap, and fails.
fn field(pieces: &[Piece], [x, y]: [f32; 2]) -> Option<f32> {
    let covering: Vec<&Piece> = pieces
        .iter()
        .filter(|piece| {
            let [left, top, right, bottom] = bounds(piece);
            x > left && x < right && y > top && y < bottom
        })
        .collect();
    assert!(covering.len() <= 1, "pieces overlap at {x}, {y}");
    let piece = covering.first()?;
    let [left, top, right, bottom] = bounds(piece);
    let (s, t) = ((x - left) / (right - left), (y - top) / (bottom - top));
    let [a, b, c, _] = piece.uv;
    let (u, v) = (a[0] + (b[0] - a[0]) * s, a[1] + (c[1] - a[1]) * t);
    Some(atlas::with(|atlas| {
        let at = |u: f32, v: f32| {
            f32::from(atlas.pixels()[v as usize * WIDTH as usize + u as usize]) / 255.0
        };
        let (u, v) = (u - 0.5, v - 0.5);
        let (fu, fv) = (u.fract(), v.fract());
        let (u, v) = (u.floor(), v.floor());
        (at(u, v) * (1.0 - fu) + at(u + 1.0, v) * fu) * (1.0 - fv)
            + (at(u, v + 1.0) * (1.0 - fu) + at(u + 1.0, v + 1.0) * fu) * fv
    }))
}

#[test]
fn a_rule_is_as_full_all_along_and_its_edges_are_where_it_says() {
    let rule = pieces(Shape::Rule {
        from: [10.0, 6.0],
        to: [100.0, 6.0],
        width: 1.0,
        color: FAINT,
    });
    // Its ends, each two pieces across, as it is thinner than the square's edge field reaches, and its middle one piece
    // across, from the wedge where it is as wide.
    assert_eq!(rule.len(), 5);
    assert!(rule[2].uv.iter().all(|uv| uv[0] == WEDGE.column(4.0)));
    assert!(rule.iter().all(|piece| piece.color == FAINT));
    // Its middle is drawn from the square's middle column, stretched, so that it is the same all along.
    let along: Vec<f32> = (12..98)
        .map(|x| field(&rule, [x as f32 + 0.37, 6.2]).unwrap())
        .collect();
    assert!(
        along.iter().all(|value| (value - along[0]).abs() < 1e-3),
        "{along:?}"
    );
    // A half on its edges, half a font pixel either side of its middle, and nothing a font pixel beyond.
    let edge = |y: f32| field(&rule, [50.0, y]).unwrap();
    assert!((edge(5.5) - 0.5).abs() < 0.02 && (edge(6.5) - 0.5).abs() < 0.02);
    assert!(edge(6.01) > 0.65);
    assert!(edge(4.52) < 0.05);
    // Its ends are soft too, at where it starts and stops.
    assert!((field(&rule, [10.0, 6.01]).unwrap() - 0.5).abs() < 0.02);
    assert!((field(&rule, [100.0, 6.01]).unwrap() - 0.5).abs() < 0.02);
    // It reaches two font pixels beyond its edges, for their antialiasing, and no further.
    let reach = rule
        .iter()
        .map(bounds)
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        });
    assert_eq!(reach, [8.0, 3.5, 102.0, 8.5]);
    // A rule with no length draws nothing, nor does a transparent one.
    assert!(
        pieces(Shape::Rule {
            from: [1.0, 1.0],
            to: [1.0, 1.0],
            width: 1.0,
            color: WHITE
        })
        .is_empty()
    );
    assert!(
        pieces(Shape::Rule {
            from: [0.0, 0.0],
            to: [9.0, 0.0],
            width: 1.0,
            color: [1.0, 1.0, 1.0, 0.0]
        })
        .is_empty()
    );
}

#[test]
fn a_slanted_rule_is_turned_along_itself() {
    let rule = pieces(Shape::Rule {
        from: [0.0, 0.0],
        to: [3.0, 4.0],
        width: 2.0,
        color: WHITE,
    });
    // Its first piece's top edge runs along it, from two font pixels before its start, two out to its side.
    let [a, b, ..] = rule[0].corners;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    assert!((dy * 3.0 - dx * 4.0).abs() < 1e-4, "{a:?} {b:?}");
    // Two font pixels back along it, and three, its half-width and margin, out to its side.
    let expected = [-0.6 * 2.0 + 0.8 * 3.0, -0.8 * 2.0 - 0.6 * 3.0];
    assert!(
        (a[0] - expected[0]).abs() < 1e-4 && (a[1] - expected[1]).abs() < 1e-4,
        "{a:?}"
    );
}

#[test]
fn a_frame_s_corners_and_sides_meet_without_overlapping_and_are_soft_inside_and_out() {
    let frame = pieces(Shape::frame(0.0, 0.0, [40.0, 20.0], 1.0, FAINT));
    assert!(frame.iter().all(|piece| piece.uv.iter().all(|[u, v]| {
        (u - FRAME.middle[0]).abs() <= FRAME.reach + REACH
            && (v - FRAME.middle[1]).abs() <= FRAME.reach + REACH
    })));
    // Its corners and sides are about as full as one another, so a translucent frame is even all round: a field read
    // between its pixels is a little less on the diagonal of a corner.
    let full = field(&frame, [20.0, 0.5]).unwrap();
    for point in [
        [0.5, 10.0],
        [39.5, 10.0],
        [20.0, 19.5],
        [0.5, 0.5],
        [39.5, 19.5],
    ] {
        assert!(
            (field(&frame, point).unwrap() - full).abs() < 0.04,
            "{point:?}"
        );
    }
    // A half at its outer edge and at its inner, a font pixel in, and empty beyond the margin inside; and hollow.
    for point in [
        [20.0, 0.0],
        [0.0, 10.0],
        [20.0, 1.0],
        [1.0, 10.0],
        [39.0, 10.0],
    ] {
        assert!(
            (field(&frame, point).unwrap() - 0.5).abs() < 0.02,
            "{point:?}"
        );
    }
    assert!(field(&frame, [20.0, 2.6]).unwrap() < 0.02);
    assert!(field(&frame, [20.0, 10.0]).is_none());
    // Sampled on a grid across it, none of its pieces overlap.
    for x in (-10..410).map(|x| x as f32 / 10.0 + 0.013) {
        for y in [-0.5, 0.3, 0.7, 5.0, 19.3, 20.2] {
            field(&frame, [x, y]);
        }
    }
}

#[test]
fn a_filled_box_is_full_inside_and_a_rounded_one_round_at_its_corners() {
    let square = pieces(Shape::rect(0.0, 0.0, 30.0, 10.0, WHITE));
    assert_eq!(square.len(), 9);
    assert_eq!(field(&square, [15.1, 5.1]), Some(1.0));
    assert!((field(&square, [0.0, 5.0]).unwrap() - 0.5).abs() < 0.02);
    let round = pieces(Shape::Rect {
        x: 0.0,
        y: 0.0,
        width: 30.0,
        height: 10.0,
        color: WHITE,
        radius: 4.0,
        outline: None,
    });
    assert!(round.iter().all(|piece| {
        let [u, v] = piece.uv[0];
        (u - DISC.middle[0]).abs() <= DISC.reach + REACH
            && (v - DISC.middle[1]).abs() <= DISC.reach + REACH
    }));
    // Where the square's corner is full, the rounded one's is outside its curve, which is a half where it passes.
    assert!(field(&square, [0.3, 0.3]).unwrap() > 0.6);
    assert!(field(&round, [0.3, 0.3]).unwrap() < 0.1);
    let on_curve = 4.0 - 4.0 / 2f32.sqrt();
    assert!((field(&round, [on_curve, on_curve]).unwrap() - 0.5).abs() < 0.03);
    assert!((field(&round, [15.0, 0.0]).unwrap() - 0.5).abs() < 0.02);
    // A radius too small for its margin is drawn square, and a box with no size draws nothing.
    let small = pieces(Shape::Rect {
        x: 0.0,
        y: 0.0,
        width: 30.0,
        height: 10.0,
        color: WHITE,
        radius: 0.5,
        outline: None,
    });
    assert_eq!(small, square);
    assert!(pieces(Shape::rect(0.0, 0.0, 0.0, 10.0, WHITE)).is_empty());
}

#[test]
fn a_fill_meets_its_neighbour_without_a_margin() {
    let fill = pieces(Shape::Fill {
        x: 1.0,
        y: 2.0,
        width: 3.0,
        height: 4.0,
        color: WHITE,
    });
    assert_eq!(fill.len(), 1);
    assert_eq!(bounds(&fill[0]), [1.0, 2.0, 4.0, 6.0]);
    assert_eq!(fill[0].uv, [SOLID; 4]);
    assert!(
        pieces(Shape::Fill {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            color: [0.0; 4]
        })
        .is_empty()
    );
}

#[test]
fn a_bar_s_fill_lies_over_its_track_along_its_longer_side() {
    let bar = |width: f32, height: f32, from: f32, to: f32| {
        pieces(Shape::Bar {
            x: 0.0,
            y: 0.0,
            width,
            height,
            track: FAINT,
            fill: WHITE,
            from,
            to,
        })
    };
    let reach = |pieces: Vec<Piece>| {
        pieces
            .iter()
            .filter(|piece| piece.color == WHITE)
            .map(bounds)
            .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            })
    };
    // Across: its fill from the start to a quarter, two font pixels' margin about it.
    assert_eq!(reach(bar(100.0, 2.0, 0.0, 0.25)), [-2.0, -2.0, 27.0, 4.0]);
    // Down: a scroll bar's thumb from a half to three quarters, its fraction kept within the track.
    assert_eq!(reach(bar(2.0, 100.0, 0.5, 2.0)), [-2.0, 48.0, 4.0, 102.0]);
    // The track is drawn first, under it.
    assert_eq!(bar(100.0, 2.0, 0.0, 0.5)[0].color, FAINT);
    // Empty, it is only its track.
    assert!(
        bar(100.0, 2.0, 0.0, 0.0)
            .iter()
            .all(|piece| piece.color == FAINT)
    );
}

#[test]
fn dots_and_rings_are_round_and_a_check_is_a_frame_with_a_tick() {
    let disc = pieces(Shape::Disc {
        centre: [10.0, 10.0],
        radius: 2.5,
        color: WHITE,
    });
    assert_eq!(field(&disc, [10.1, 10.1]), Some(1.0));
    assert!((field(&disc, [12.5, 10.01]).unwrap() - 0.5).abs() < 0.03);
    let diagonal = 2.5 / 2f32.sqrt();
    assert!((field(&disc, [10.0 + diagonal, 10.0 - diagonal]).unwrap() - 0.5).abs() < 0.03);
    let ring = pieces(Shape::Ring {
        centre: [10.0, 10.0],
        radius: 2.6,
        color: WHITE,
    });
    assert_eq!(ring.len(), 1);
    assert!(field(&ring, [10.1, 10.1]).unwrap() < 0.01);
    assert!(field(&ring, [10.0 + 2.6 * 5.0 / 6.0, 10.01]).unwrap() > 0.6);
    assert!(
        pieces(Shape::Ring {
            centre: [0.0; 2],
            radius: 0.5,
            color: WHITE
        })
        .is_empty()
    );
    let check = |ticked| {
        pieces(Shape::Check {
            x: 0.0,
            y: 0.0,
            size: 8.0,
            ticked,
            color: WHITE,
        })
    };
    let empty = check(false);
    assert_eq!(
        empty,
        pieces(Shape::frame(0.0, 0.0, [8.0, 8.0], 1.2, WHITE))
    );
    // Hollow, and soft at its edge.
    assert!(field(&empty, [4.01, 4.01]).is_none());
    assert!(field(&empty, [2.5, 4.01]).unwrap() < 0.02);
    assert!((field(&empty, [0.0, 4.01]).unwrap() - 0.5).abs() < 0.02);
    // Too small for its band and margins on both sides, a frame's sides meet at its middle, which is hollow.
    let small = pieces(Shape::frame(0.0, 0.0, [4.0, 4.0], 1.0, WHITE));
    let middle = field(&small, [2.01, 2.01]).unwrap();
    assert!(middle < 0.06, "{middle}");
    // A frame with no width draws nothing.
    assert!(pieces(Shape::frame(0.0, 0.0, [8.0, 8.0], 0.0, WHITE)).is_empty());
    // Ticked, it is filled, with a dark tick over it.
    let ticked = check(true);
    let filled = pieces(Shape::rect(0.0, 0.0, 8.0, 8.0, WHITE));
    assert_eq!(&ticked[..filled.len()], filled.as_slice());
    let tick = &ticked[filled.len()..];
    assert!(!tick.is_empty() && tick.iter().all(|piece| piece.color == TICK_INK));
    // The tick's strokes stay inside the box, but for the margins of their edges.
    let [_, _, right, bottom] =
        tick.iter()
            .map(bounds)
            .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            });
    assert!(right < 10.0 && bottom < 10.0, "{right} {bottom}");
}

#[test]
fn a_piece_cut_into_strips_draws_the_same() {
    let rule = pieces(Shape::Rule {
        from: [0.0, 0.0],
        to: [95.0, 0.0],
        width: 1.0,
        color: WHITE,
    });
    let middle = rule
        .iter()
        .max_by(|a, b| (bounds(a)[2] - bounds(a)[0]).total_cmp(&(bounds(b)[2] - bounds(b)[0])))
        .unwrap();
    let strips = middle.strips(10.0);
    let length = bounds(middle)[2] - bounds(middle)[0];
    assert_eq!(strips.len(), (length / 10.0).ceil() as usize);
    assert_eq!(strips[0].corners[0], middle.corners[0]);
    assert_eq!(strips.last().unwrap().corners[3], middle.corners[3]);
    assert!(
        strips
            .iter()
            .all(|strip| strip.uv[0] == middle.uv[0] && strip.color == WHITE)
    );
    assert_eq!(middle.strips(1000.0), vec![*middle]);
}

#[test]
fn marks_stand_in_for_their_shapes_and_join_as_lines_do() {
    let marks = [
        (Mark::Rule, '─', Join::Across),
        (Mark::Line, '▏', Join::Down),
        (Mark::Quote, '│', Join::Down),
        (Mark::Check { ticked: false }, '☐', Join::Alone),
        (Mark::Check { ticked: true }, '☑', Join::Alone),
        (Mark::Dot, '●', Join::Alone),
        (Mark::Ring { dot: false }, '○', Join::Alone),
        (Mark::Ring { dot: true }, '◉', Join::Alone),
    ];
    for (mark, stand_in, join) in marks {
        assert_eq!(mark.stand_in(), stand_in);
        assert_eq!(mark.joins(), join);
        // Each character is in a bundled font, so that the console and anything printing it shows it.
        assert!(tomori_text::fonts::covers(stand_in), "{stand_in}");
        let span = mark.span(3, Style::plain(WHITE));
        assert_eq!(span.text, String::from(stand_in).repeat(3));
        assert_eq!(span.style.mark, Some(mark));
        assert_eq!(mark.row(2, Style::plain(WHITE)).cells.len(), 2);
    }
    // A rule across three cells, through their middles; a line down two rows, near their left edge; a quote's bar
    // down their middles; and a dot about the middle of a lowercase letter.
    let rule = Mark::Rule.shapes(9.0, 12.0, 3, WHITE);
    assert_eq!(
        rule,
        [Shape::Rule {
            from: [9.0, 18.0],
            to: [36.0, 18.0],
            width: 1.0,
            color: WHITE
        }]
    );
    assert_eq!(
        Mark::Line.shapes(0.0, 0.0, 2, WHITE),
        [Shape::Rule {
            from: [0.7, 0.0],
            to: [0.7, 24.0],
            width: 1.0,
            color: WHITE
        }]
    );
    assert_eq!(
        Mark::Quote.shapes(0.0, 0.0, 1, WHITE),
        [Shape::Rule {
            from: [4.5, 0.0],
            to: [4.5, 12.0],
            width: 1.5,
            color: WHITE
        }]
    );
    assert!(matches!(
        Mark::Check { ticked: true }.shapes(0.0, 0.0, 1, WHITE)[..],
        [Shape::Check {
            ticked: true,
            size: 8.0,
            ..
        }]
    ));
    assert!(matches!(
        Mark::Dot.shapes(0.0, 0.0, 1, WHITE)[..],
        [Shape::Disc {
            centre: [4.5, 5.9],
            ..
        }]
    ));
    assert!(matches!(
        Mark::Ring { dot: false }.shapes(0.0, 0.0, 1, WHITE)[..],
        [Shape::Ring { .. }]
    ));
    assert!(matches!(
        Mark::Ring { dot: true }.shapes(0.0, 0.0, 1, WHITE)[..],
        [Shape::Ring { .. }, Shape::Disc { radius: 1.3, .. }]
    ));
}
