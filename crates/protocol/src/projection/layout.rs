// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where a wall's slots stand: one layout that the server, which lays panels out and tells the agent where they are,
//! and every client, which draws and aims at them, share, so that the two cannot drift apart. See
//! `design/projection.md`.
//!
//! A flat wall is measured as a headset draws it, in font pixels of [`PIXEL`]: its slots are a panel's size, with a
//! gutter between them and around them, below a row for its label, and the board is a whole number of text rows
//! tall. A curved wall has no board: its slots stand along an arc, in rows, each slot turned to face the line the arc
//! curves about. Either is scaled about its pose by its [`super::Wall::scale`].
use serde::{Deserialize, Serialize};

use super::{Wall, WallShape};
use crate::Pose;

/// The font pixel a wall is measured in at scale 1, in metres.
pub const PIXEL: f32 = 0.0019;

/// A slot's width and height at scale 1, in font pixels: a panel's face of 34 columns of 9 pixels and 15 rows of 12,
/// padded.
const SLOT_PIXELS: (u32, u32) = (318, 190);

/// The gutter between a flat wall's slots, and around them, in font pixels.
const GUTTER_PIXELS: u32 = 24;

/// From a flat wall's top edge to its first row of slots, in font pixels: its padding, its label's row and a gap.
const TOP_PIXELS: u32 = 27;

/// A flat wall's padding, and the height of the text rows its board is a whole number of, in font pixels.
const BOARD_PADDING: u32 = 3;
const BOARD_ROW: u32 = 12;

/// A slot's width and height at scale 1, in metres: a panel's.
pub const SLOT_SIZE: (f32, f32) = (SLOT_PIXELS.0 as f32 * PIXEL, SLOT_PIXELS.1 as f32 * PIXEL);

/// The gutter between a flat wall's slots at scale 1, in metres.
pub const GUTTER: f32 = GUTTER_PIXELS as f32 * PIXEL;

/// How large a curved wall's slots are unless it says otherwise, as a proportion of a flat wall's: the Tomori wall's,
/// which stands nearer than a flat wall usually does.
pub const ARC_SCALE: f32 = 0.7;

/// The gap between a curved wall's slots, across and down, at [`ARC_SCALE`], in metres.
pub const ARC_GAP: f32 = 0.03;

/// How far in front of its wall a slot's panel stands, in metres, so that it is drawn over the wall.
pub const LIFT: f32 = 0.01;

/// How many columns and rows of slots a wall has unless it says otherwise: three columns of two rows when flat, and
/// six in one row when curved.
pub const FLAT_GRID: Grid = Grid {
    columns: 3,
    rows: 2,
};
pub const ARC_GRID: Grid = Grid {
    columns: 6,
    rows: 1,
};

/// The most columns, rows and slots one wall has.
pub const MOST_COLUMNS: u32 = 8;
pub const MOST_ROWS: u32 = 4;
pub const MOST_SLOTS: u32 = 24;

/// The least and most a wall's slots are scaled: the size of the text in them, as a proportion of a standard slot's.
pub const SCALES: (f32, f32) = (0.5, 2.0);

/// A standard slot's width over its height, about 5:3.
pub const STANDARD_ASPECT: f32 = SLOT_PIXELS.0 as f32 / SLOT_PIXELS.1 as f32;

/// The narrowest and widest a wall's slots are, as their width over their height: from 1:4, tall, to 4:1, wide.
pub const ASPECTS: (f32, f32) = (0.25, 4.0);

/// The least radius a wall is curved at, in metres, and the most of a circle its columns may span, in radians: half of
/// one. A wall sent curved more tightly, or around more, is drawn flat.
pub const LEAST_RADIUS: f32 = 1.0;
pub const MOST_ARC: f32 = std::f32::consts::PI;

/// A wall's slots: so many columns of so many rows, numbered from the top left as seen from in front, left to right
/// and then down. Sent as `[columns, rows]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "(u32, u32)", into = "(u32, u32)")]
#[non_exhaustive]
pub struct Grid {
    pub columns: u32,
    pub rows: u32,
}

impl From<(u32, u32)> for Grid {
    fn from((columns, rows): (u32, u32)) -> Self {
        Self { columns, rows }
    }
}

impl From<Grid> for (u32, u32) {
    fn from(grid: Grid) -> Self {
        (grid.columns, grid.rows)
    }
}

impl Grid {
    /// `columns` of `rows`, if a wall can have them: at least one of each, at most [`MOST_COLUMNS`] and [`MOST_ROWS`],
    /// and at most [`MOST_SLOTS`] in all.
    pub fn new(columns: u32, rows: u32) -> Option<Self> {
        let grid = Self { columns, rows };
        grid.valid().then_some(grid)
    }

    /// Whether a wall can have it.
    pub fn valid(&self) -> bool {
        (1..=MOST_COLUMNS).contains(&self.columns)
            && (1..=MOST_ROWS).contains(&self.rows)
            && self.columns * self.rows <= MOST_SLOTS
    }

    /// How many slots it has.
    pub fn slots(&self) -> u32 {
        self.columns * self.rows
    }

    /// The column and row of `slot`, if it has it.
    pub fn cell(&self, slot: u32) -> Option<(u32, u32)> {
        (slot < self.slots()).then(|| (slot % self.columns, slot / self.columns))
    }

    /// The slot at `column` and `row`, if it has it.
    pub fn slot(&self, column: u32, row: u32) -> Option<u32> {
        (column < self.columns && row < self.rows).then(|| row * self.columns + column)
    }
}

/// Where one of a wall's slots stands in the wall's own frame, that of its pose, as the middle of a panel in it: its
/// position, in metres, how far it is turned about the vertical, in radians, anticlockwise as seen from above, how
/// large its text is, as a proportion of a standard slot's, and its width and height, in metres, which a slot of a
/// wall's own [`Wall::aspect`] makes wider or taller than [`SLOT_SIZE`] at that scale.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SlotFrame {
    pub position: [f32; 3],
    pub yaw: f32,
    pub scale: f32,
    pub size: [f32; 2],
}

impl SlotFrame {
    /// Where it stands in the space, for a wall at `wall`: its middle, facing out of the wall.
    pub fn pose(&self, wall: &Pose) -> Pose {
        let (sin, cos) = (self.yaw / 2.0).sin_cos();
        Pose {
            position: add(wall.position, rotate(wall.orientation, self.position)),
            orientation: multiply(wall.orientation, [0.0, sin, 0.0, cos]),
        }
    }
}

/// `vector` turned by the unit quaternion `rotation`.
pub(crate) fn rotate([x, y, z, w]: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let length = (x * x + y * y + z * z + w * w).sqrt();
    let [x, y, z, w] = if length > 0.0 {
        [x / length, y / length, z / length, w / length]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    };
    let [vx, vy, vz] = vector;
    // t = 2 (q × v); v' = v + w t + q × t.
    let (tx, ty, tz) = (
        2.0 * (y * vz - z * vy),
        2.0 * (z * vx - x * vz),
        2.0 * (x * vy - y * vx),
    );
    [
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    ]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The quaternion `a` then `b`, as `a * b`.
fn multiply([ax, ay, az, aw]: [f32; 4], [bx, by, bz, bw]: [f32; 4]) -> [f32; 4] {
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// A flat wall's board at scale 1, `grid` of slots each `slot` font pixels wide and tall, in font pixels: as wide as its
/// slots and the gutters between and around them, and as tall as its label, its rows of slots and a gutter below them,
/// rounded up to whole rows of text.
fn board_pixels(grid: Grid, slot: (f32, f32)) -> (f32, f32) {
    let gutter = GUTTER_PIXELS as f32;
    let width = gutter + grid.columns as f32 * (slot.0 + gutter);
    let content = TOP_PIXELS as f32 + grid.rows as f32 * (slot.1 + gutter);
    let rows = ((content - 2.0 * BOARD_PADDING as f32) / BOARD_ROW as f32 - 1e-3).ceil();
    (width, rows * BOARD_ROW as f32 + 2.0 * BOARD_PADDING as f32)
}

impl Wall {
    /// The shape of its slots, their width over their height: as it says, within [`ASPECTS`], or else a standard
    /// slot's, [`STANDARD_ASPECT`].
    pub fn aspect(&self) -> f32 {
        match self.aspect {
            Some(aspect) if aspect.is_finite() && aspect > 0.0 => {
                aspect.clamp(ASPECTS.0, ASPECTS.1)
            }
            _ => STANDARD_ASPECT,
        }
    }

    /// Its slots' width and height at scale 1, in font pixels: a standard slot's, made wider or taller to its
    /// [`Wall::aspect`], so that its text is the size its scale says and it holds more columns or rows. A wider slot is
    /// a whole number of text cells wider, so that a flat wall's board stays a whole number of them.
    fn slot_pixels(&self) -> (f32, f32) {
        let (width, height) = (SLOT_PIXELS.0 as f32, SLOT_PIXELS.1 as f32);
        let aspect = self.aspect();
        if (aspect - STANDARD_ASPECT).abs() < 1e-4 {
            return (width, height);
        }
        if aspect > STANDARD_ASPECT {
            let wide = height * aspect;
            // A gutter and a slot are a whole number of cells of 9 font pixels, as a standard one is.
            let cells = ((wide + GUTTER_PIXELS as f32) / 9.0).round();
            (cells * 9.0 - GUTTER_PIXELS as f32, height)
        } else {
            (width, width / aspect)
        }
    }

    /// Its slots' width and height, in metres: [`SLOT_SIZE`] at its scale, made wider or taller to its aspect.
    pub fn slot_size(&self) -> (f32, f32) {
        let (width, height) = self.slot_pixels();
        let scale = self.scale();
        (width * PIXEL * scale, height * PIXEL * scale)
    }

    /// A flat wall's board, at scale 1, in text cells of 9 by 12 font pixels inside its padding of [`BOARD_PADDING`]:
    /// its columns and rows, as a headset's canvas draws it.
    pub fn board_cells(&self) -> (usize, usize) {
        let (width, height) = board_pixels(self.grid(), self.slot_pixels());
        let padding = 2.0 * BOARD_PADDING as f32;
        (
            ((width - padding) / 9.0).round() as usize,
            ((height - padding) / BOARD_ROW as f32).round() as usize,
        )
    }

    /// The left and top edges of `slot`'s place on its flat board, at scale 1, in font pixels from its top left corner,
    /// and its width and height.
    pub fn slot_rect(&self, slot: u32) -> (f32, f32, f32, f32) {
        let (column, row) = self.grid().cell(slot).unwrap_or_default();
        let (width, height) = self.slot_pixels();
        let gutter = GUTTER_PIXELS as f32;
        (
            gutter + column as f32 * (width + gutter),
            TOP_PIXELS as f32 + row as f32 * (height + gutter),
            width,
            height,
        )
    }

    /// Whether its shape is an arc it could be curved along, of a radius of at least [`LEAST_RADIUS`], so that it
    /// takes a curved wall's grid and scale unless it says otherwise. A shape it cannot be curved along takes a flat
    /// wall's, as it is drawn flat.
    fn arc_shaped(&self) -> bool {
        matches!(self.shape, WallShape::Arc { radius } if radius.is_finite() && radius >= LEAST_RADIUS)
    }

    /// Its grid of slots: as it says, if a wall can have it, or else its shape's.
    pub fn grid(&self) -> Grid {
        match self.grid {
            Some(grid) if grid.valid() => grid,
            _ if self.arc_shaped() => ARC_GRID,
            _ => FLAT_GRID,
        }
    }

    /// How many slots it has.
    pub fn slots(&self) -> u32 {
        self.grid().slots()
    }

    /// How large its slots are, as a proportion of [`SLOT_SIZE`]: as it says, within [`SCALES`], or else its shape's.
    pub fn scale(&self) -> f32 {
        match self.scale {
            Some(scale) if scale.is_finite() => scale.clamp(SCALES.0, SCALES.1),
            _ if self.arc_shaped() => ARC_SCALE,
            _ => 1.0,
        }
    }

    /// The angle between the middles of neighbouring columns of a wall curved at `radius`, in radians.
    fn pitch(&self, radius: f32) -> f32 {
        let scale = self.scale();
        (self.slot_size().0 + ARC_GAP * scale / ARC_SCALE) / radius
    }

    /// The radius its slots are drawn curved at, if it is curved: at least [`LEAST_RADIUS`], with its columns spanning
    /// at most [`MOST_ARC`]. Any other shape is drawn flat.
    pub fn radius(&self) -> Option<f32> {
        match self.shape {
            WallShape::Arc { radius }
                if self.arc_shaped()
                    && self.pitch(radius) * self.grid().columns as f32 <= MOST_ARC + 1e-4 =>
            {
                Some(radius)
            }
            _ => None,
        }
    }

    /// Its flat board's width and height, in metres, scaled, if it is drawn flat.
    pub fn board(&self) -> Option<(f32, f32)> {
        if self.radius().is_some() {
            return None;
        }
        let (width, height) = board_pixels(self.grid(), self.slot_pixels());
        let scale = self.scale();
        Some((width * PIXEL * scale, height * PIXEL * scale))
    }

    /// How wide and tall it is, in metres: a flat wall's board, or the arc a curved wall's slots span, measured along
    /// it, and their rows.
    pub fn size(&self) -> (f32, f32) {
        if let Some(board) = self.board() {
            return board;
        }
        let scale = self.scale();
        let grid = self.grid();
        let gap = ARC_GAP * scale / ARC_SCALE;
        let (slot_width, slot_height) = self.slot_size();
        (
            grid.columns as f32 * (slot_width + gap) - gap,
            grid.rows as f32 * (slot_height + gap) - gap,
        )
    }

    /// Where each of its slots stands, in its own frame, numbered as its [`Grid`] numbers them. A flat wall's stand in
    /// its plane, just in front of its board; a curved wall's along its arc, each turned to face the line the arc curves
    /// about, its rows one above another about its pose.
    pub fn slot_frames(&self) -> Vec<SlotFrame> {
        let grid = self.grid();
        let scale = self.scale();
        let (slot_width, slot_height) = self.slot_size();
        let size = [slot_width, slot_height];
        let Some(radius) = self.radius() else {
            let (width, height) = board_pixels(grid, self.slot_pixels());
            return (0..grid.slots())
                .map(|slot| {
                    let (left, top, across, down) = self.slot_rect(slot);
                    let x = (left + across / 2.0 - width / 2.0) * PIXEL;
                    let y = (height / 2.0 - top - down / 2.0) * PIXEL;
                    SlotFrame {
                        position: [x * scale, y * scale, LIFT],
                        yaw: 0.0,
                        scale,
                        size,
                    }
                })
                .collect();
        };
        let pitch = self.pitch(radius);
        let down = slot_height + ARC_GAP * scale / ARC_SCALE;
        (0..grid.slots())
            .map(|slot| {
                let (column, row) = grid.cell(slot).unwrap_or_default();
                let angle = (column as f32 - (grid.columns as f32 - 1.0) / 2.0) * pitch;
                let y = ((grid.rows as f32 - 1.0) / 2.0 - row as f32) * down;
                // Its middle on the arc, then lifted along its own facing, turned by -angle about the vertical.
                let (sin, cos) = angle.sin_cos();
                SlotFrame {
                    position: [
                        radius * sin - LIFT * sin,
                        y,
                        radius * (1.0 - cos) + LIFT * cos,
                    ],
                    yaw: -angle,
                    scale,
                    size,
                }
            })
            .collect()
    }

    /// Where `slot` stands in the space, if the wall has it: its middle, facing out of the wall.
    pub fn slot_pose(&self, slot: u32) -> Option<Pose> {
        self.slot_frames()
            .get(slot as usize)
            .map(|frame| frame.pose(&self.pose))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TOMORI_WALL, WallKind};

    fn level() -> Pose {
        Pose {
            position: [0.0, 1.5, -2.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn a_grid_is_bounded_and_numbers_its_slots_from_the_top_left() {
        assert_eq!(Grid::new(4, 1), Some(Grid::from((4, 1))));
        assert_eq!(Grid::new(0, 1), None);
        assert_eq!(Grid::new(1, 0), None);
        assert_eq!(Grid::new(MOST_COLUMNS + 1, 1), None);
        assert_eq!(Grid::new(1, MOST_ROWS + 1), None);
        assert_eq!(Grid::new(8, 4), None);
        assert!(Grid::new(6, 4).is_some());
        let grid = Grid::new(4, 2).unwrap();
        assert_eq!(grid.slots(), 8);
        assert_eq!(grid.cell(5), Some((1, 1)));
        assert_eq!(grid.cell(8), None);
        assert_eq!(grid.slot(3, 1), Some(7));
        assert_eq!(grid.slot(4, 0), None);
        assert_eq!(grid.slot(0, 2), None);
        assert_eq!(<(u32, u32)>::from(grid), (4, 2));
        assert_eq!(
            serde_json::to_value(grid).unwrap(),
            serde_json::json!([4, 2])
        );
    }

    #[test]
    fn a_flat_wall_is_laid_out_as_before_unless_it_says_otherwise() {
        let wall = Wall::new("wall-1", WallKind::Added, level());
        assert_eq!(wall.grid(), FLAT_GRID);
        assert_eq!(wall.scale(), 1.0);
        let (width, height) = wall.board().unwrap();
        // About 2 m by 0.9 m, as the flat wall has always been drawn.
        assert!((width - 1.995).abs() < 1e-4 && (height - 0.8778).abs() < 1e-4);
        assert_eq!(wall.size(), (width, height));
        let frames = wall.slot_frames();
        assert_eq!(frames.len(), 6);
        let at = |slot: usize| frames[slot].position;
        assert!((at(0)[0] + 0.6498).abs() < 1e-4 && (at(0)[1] - 0.2071).abs() < 1e-4);
        assert!(at(1)[0].abs() < 1e-4 && (at(4)[1] + 0.1995).abs() < 1e-4);
        assert!((at(5)[0] - 0.6498).abs() < 1e-4);
        assert!(
            frames
                .iter()
                .all(|frame| frame.yaw == 0.0 && frame.scale == 1.0 && frame.position[2] == LIFT)
        );
        assert_eq!(wall.board_cells(), (116, 38));
    }

    #[test]
    fn a_flat_wall_of_any_grid_holds_its_slots_inside_its_board_and_scales_about_its_pose() {
        for grid in [(1, 1), (4, 1), (8, 3), (6, 4), (2, 4)] {
            let grid = Grid::from(grid);
            for scale in [0.5, 1.0, 1.6, 2.0] {
                let wall = Wall::new("w", WallKind::Added, level())
                    .with_grid(grid)
                    .with_scale(scale);
                let (width, height) = wall.board().unwrap();
                let frames = wall.slot_frames();
                assert_eq!(frames.len() as u32, grid.slots());
                let (slot_width, slot_height) = (SLOT_SIZE.0 * scale, SLOT_SIZE.1 * scale);
                for (slot, frame) in frames.iter().enumerate() {
                    let [x, y, _] = frame.position;
                    // Within the board, a gutter from its sides and bottom, below its label.
                    assert!(x.abs() + slot_width / 2.0 <= width / 2.0 - GUTTER * scale + 1e-4);
                    assert!(y - slot_height / 2.0 >= -height / 2.0 + GUTTER * scale - 1e-4);
                    assert!(
                        y + slot_height / 2.0
                            <= height / 2.0 - TOP_PIXELS as f32 * PIXEL * scale + 1e-4
                    );
                    // Left to right, then down.
                    let (column, row) = grid.cell(slot as u32).unwrap();
                    if column > 0 {
                        let left = frames[slot - 1].position;
                        assert!((x - left[0] - (slot_width + GUTTER * scale)).abs() < 1e-4);
                    }
                    if row > 0 {
                        let above = frames[slot - grid.columns as usize].position;
                        assert!((above[1] - y - (slot_height + GUTTER * scale)).abs() < 1e-4);
                    }
                }
                // Its board is a whole number of rows of text, so that a headset draws it exactly.
                let (columns, rows) = wall.board_cells();
                assert!((columns as f32 * 9.0 + 6.0) * PIXEL * scale - width < 1e-4);
                assert!(((rows as f32 * 12.0 + 6.0) * PIXEL * scale - height).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn a_wall_s_slots_take_any_shape_their_text_its_scale_and_the_slot_taller_or_wider() {
        let wall = |aspect: Option<f32>| {
            Wall::new("w", WallKind::Added, level())
                .with_grid(Grid::from((3, 1)))
                .with_scale(1.0)
                .with_aspect(aspect)
        };
        // Unsaid, or nonsense, its slots are a standard slot's shape, laid out as before.
        let standard = wall(None);
        assert_eq!(standard.aspect(), STANDARD_ASPECT);
        assert_eq!(standard.slot_size(), SLOT_SIZE);
        for nonsense in [f32::NAN, 0.0, -1.0, f32::INFINITY] {
            assert_eq!(wall(Some(nonsense)).slot_frames(), standard.slot_frames());
        }
        // Tall, 9:16: as wide as a standard slot and taller, so that its text is the same size and it holds more rows.
        let tall = wall(Some(9.0 / 16.0));
        let (width, height) = tall.slot_size();
        assert!((width - SLOT_SIZE.0).abs() < 1e-5);
        assert!((width / height - 9.0 / 16.0).abs() < 1e-4);
        let frames = tall.slot_frames();
        assert!(
            frames
                .iter()
                .all(|frame| frame.size == [width, height] && frame.scale == 1.0)
        );
        // Its board holds them, a gutter apart, below its label, a whole number of rows of text tall.
        let (board_width, board_height) = tall.board().unwrap();
        for frame in &frames {
            let [x, y, _] = frame.position;
            assert!(x.abs() + width / 2.0 <= board_width / 2.0 - GUTTER + 1e-4);
            assert!(y - height / 2.0 >= -board_height / 2.0 + GUTTER - 1e-4);
            assert!(y + height / 2.0 <= board_height / 2.0 - TOP_PIXELS as f32 * PIXEL + 1e-4);
        }
        let (columns, rows) = tall.board_cells();
        assert!(((columns * 9 + 6) as f32 * PIXEL - board_width).abs() < 1e-4);
        assert!(((rows * 12 + 6) as f32 * PIXEL - board_height).abs() < 1e-4);
        // Wide, 16:9 and 4:1: as tall as a standard slot and a whole number of cells wider.
        for aspect in [16.0 / 9.0, 4.0, 3.0] {
            let wide = wall(Some(aspect)).with_scale(1.4);
            let (width, height) = wide.slot_size();
            assert!((height - SLOT_SIZE.1 * 1.4).abs() < 1e-5);
            let shape = width / height;
            assert!((shape - aspect).abs() < 0.03, "{aspect}: {shape}");
            let (columns, _) = wide.board_cells();
            let (board_width, _) = wide.board().unwrap();
            assert!(((columns * 9 + 6) as f32 * PIXEL * 1.4 - board_width).abs() < 1e-4);
        }
        // Beyond its bounds, as narrow or as wide as a slot can be.
        assert_eq!(wall(Some(0.01)).aspect(), ASPECTS.0);
        assert_eq!(wall(Some(40.0)).aspect(), ASPECTS.1);
        // Curved, its rows of tall slots stand along the arc, as far apart as they are wide.
        let curved = tall.clone().with_shape(WallShape::Arc { radius: 3.0 });
        let (width, height) = curved.slot_size();
        let frames = curved.slot_frames();
        assert_eq!(frames.len(), 3);
        assert!(frames.iter().all(|frame| frame.size == [width, height]));
        let (span, tall_span) = curved.size();
        let gap = ARC_GAP / ARC_SCALE;
        assert!((span - (3.0 * (width + gap) - gap)).abs() < 1e-4);
        assert!((tall_span - height).abs() < 1e-5);
    }

    #[test]
    fn a_curved_wall_stands_its_rows_along_its_arc_each_slot_facing_the_line_it_curves_about() {
        let tomori = Wall::new(TOMORI_WALL, WallKind::Tomori, level())
            .with_shape(WallShape::Arc { radius: 2.3 });
        assert_eq!(tomori.grid(), ARC_GRID);
        assert_eq!(tomori.scale(), ARC_SCALE);
        assert_eq!(tomori.radius(), Some(2.3));
        assert_eq!(tomori.board(), None);
        let frames = tomori.slot_frames();
        // Its slots as the Tomori wall has always stood: 0.7 of a flat slot, 3 cm apart along the arc.
        let pitch = (SLOT_SIZE.0 * ARC_SCALE + ARC_GAP) / 2.3;
        for (slot, frame) in frames.iter().enumerate() {
            let angle = (slot as f32 - 2.5) * pitch;
            assert!((frame.yaw + angle).abs() < 1e-5);
            assert_eq!(frame.position[1], 0.0);
            // Each faces the point 2.3 m ahead of the wall's middle.
            let [x, _, z] = frame.position;
            let distance = (x * x + (z - 2.3) * (z - 2.3)).sqrt();
            assert!((distance - (2.3 - LIFT)).abs() < 1e-4, "{distance}");
        }
        let (width, height) = tomori.size();
        assert!((width - (6.0 * (SLOT_SIZE.0 * ARC_SCALE + ARC_GAP) - ARC_GAP)).abs() < 1e-4);
        assert!((height - SLOT_SIZE.1 * ARC_SCALE).abs() < 1e-5);
        // Two rows stand one above the other about its middle, each curved the same.
        let two = Wall::new("w", WallKind::Added, level())
            .with_shape(WallShape::Arc { radius: 3.0 })
            .with_grid(Grid::from((4, 2)))
            .with_scale(1.0);
        let frames = two.slot_frames();
        assert_eq!(frames.len(), 8);
        assert!(
            frames[0].position[1] > 0.0
                && (frames[0].position[1] + frames[4].position[1]).abs() < 1e-5
        );
        assert!((frames[0].yaw - frames[4].yaw).abs() < 1e-6);
        assert!(
            (frames[0].position[1] - frames[4].position[1] - (SLOT_SIZE.1 + ARC_GAP / ARC_SCALE))
                .abs()
                < 1e-4
        );
    }

    #[test]
    fn a_wall_curved_too_tightly_or_around_too_much_or_of_an_unknown_shape_is_flat() {
        let curved = |radius: f32, columns: u32| {
            Wall::new("w", WallKind::Added, level())
                .with_shape(WallShape::Arc { radius })
                .with_grid(Grid::from((columns, 1)))
                .with_scale(1.0)
        };
        assert_eq!(curved(0.9, 1).radius(), None);
        // One that says neither grid nor scale is drawn as a flat wall is, as before.
        let tight =
            Wall::new("w", WallKind::Added, level()).with_shape(WallShape::Arc { radius: 0.2 });
        assert_eq!(
            (tight.grid(), tight.scale(), tight.radius()),
            (FLAT_GRID, 1.0, None)
        );
        assert_eq!(curved(f32::NAN, 1).radius(), None);
        assert_eq!(curved(f32::INFINITY, 1).radius(), None);
        // Eight columns of slots 65 cm apart span more than half a circle at 1.5 m, but not at 2 m.
        assert_eq!(curved(1.5, 8).radius(), None);
        assert_eq!(curved(2.0, 8).radius(), Some(2.0));
        assert!(curved(1.5, 8).board().is_some());
        let unknown = Wall::new("w", WallKind::Added, level()).with_shape(WallShape::Unknown);
        assert_eq!(unknown.grid(), FLAT_GRID);
        assert!(unknown.board().is_some());
    }

    #[test]
    fn a_wall_saying_a_grid_or_scale_it_cannot_have_is_drawn_as_its_shape_says() {
        let wall = Wall::new("w", WallKind::Added, level())
            .with_grid(Grid::from((100, 100)))
            .with_scale(f32::NAN);
        assert_eq!((wall.grid(), wall.scale()), (FLAT_GRID, 1.0));
        let wall = wall.with_scale(40.0);
        assert_eq!(wall.scale(), SCALES.1);
        let wall = wall.with_scale(0.01);
        assert_eq!(wall.scale(), SCALES.0);
    }

    #[test]
    fn a_slot_stands_in_the_space_where_its_wall_stands_it() {
        // A wall turned a quarter about the vertical, to face +X.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let pose = Pose {
            position: [1.0, 1.5, -2.0],
            orientation: [0.0, half, 0.0, half],
        };
        let wall = Wall::new("w", WallKind::Added, pose);
        let frames = wall.slot_frames();
        let slot = wall.slot_pose(0).unwrap();
        // Its left, as seen from in front, is towards +Z once the wall faces +X; its lift is along +X.
        let [x, y, _] = frames[0].position;
        assert!((slot.position[0] - (1.0 + LIFT)).abs() < 1e-5);
        assert!((slot.position[1] - (1.5 + y)).abs() < 1e-5);
        assert!(
            x < 0.0 && (slot.position[2] - (-2.0 - x)).abs() < 1e-5,
            "{slot:?}"
        );
        assert_eq!(slot.orientation, pose.orientation);
        assert_eq!(wall.slot_pose(6), None);
        // A curved slot is turned by its own yaw, after the wall's.
        let curved =
            Wall::new("w", WallKind::Added, level()).with_shape(WallShape::Arc { radius: 2.0 });
        let first = curved.slot_frames()[0];
        let pose = curved.slot_pose(0).unwrap();
        let (sin, cos) = (first.yaw / 2.0).sin_cos();
        assert!(
            (pose.orientation[1] - sin).abs() < 1e-6 && (pose.orientation[3] - cos).abs() < 1e-6
        );
        // A degenerate rotation is taken as none.
        assert_eq!(rotate([0.0; 4], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    }
}
