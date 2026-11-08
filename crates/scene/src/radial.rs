// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A radial layout of items grouped into sectors: the circle around a centre is divided into one sector for each group,
//! such as a folder of sessions, in proportion to how many items each holds, and each sector's items stand further from
//! the centre the older they are, in rings of age, today, this week, this month and older, each outside the last, with
//! the sector's title at its outer edge. See `design/arrival.md`.
//!
//! It knows nothing of the world or of what the items are: everything is in degrees as seen from the eye, across and up
//! from the centre of the view the layout fills, so that the sizes the text rules ask for (see `design/text.md`) hold at
//! any distance. [`crate::session_browser`] uses it for sessions and [`crate::arrival`] places the result in the world;
//! items whose age is not known, such as spaces, all stand in one ring.
use glam::Vec2;

use crate::panel::Rect;

/// A font pixel, in degrees as seen from the eye: an x-height of about half a degree, a little above the 0.45° that
/// `design/text.md` asks for, so that text towards the edge of the layout still reads comfortably.
pub const PIXEL: f32 = 0.086;
/// The height of a row of text with its padding, and the clear room kept around each chip and title.
pub const LINE: f32 = 15.0 * PIXEL;
pub const GAP: f32 = 0.45;
/// The most rows a chip's label takes: a label longer than a row wraps onto a second.
pub const ROWS: usize = 2;
/// The ages at which the rings change: a day, a week and about a month, in seconds.
pub const BANDS: [u64; 3] = [86_400, 7 * 86_400, 30 * 86_400];
/// How finely the layout searches outwards and along each ring, in degrees.
const RADIAL_STEP: f32 = 0.3;
const ARC_STEP: f32 = 0.5;

/// The width of a chip whose label takes `columns` cells: its marker, then its label, with padding.
pub fn chip_width(columns: usize) -> f32 {
    (9.0 * columns as f32 + 24.0) * PIXEL
}

/// The height of a chip of `rows` rows.
pub fn chip_height(rows: usize) -> f32 {
    (11.0 * rows as f32 + 4.0) * PIXEL
}

/// The width of a title of `columns` cells, which has no marker.
pub fn title_width(columns: usize) -> f32 {
    (9.0 * columns as f32 + 6.0) * PIXEL
}

/// A chip for a label of `wanted` cells where a row shows at most `most`: its size, and the cells each row shows and
/// how many rows.
pub fn chip(wanted: usize, most: usize) -> (Vec2, usize, usize) {
    let (columns, rows) = if wanted <= most {
        (wanted.max(1), 1)
    } else {
        (most, ROWS)
    };
    (
        Vec2::new(chip_width(columns), chip_height(rows)),
        columns,
        rows,
    )
}

/// An item to lay out. Items are given in their order, the first nearest the centre.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The group it belongs to, by its index in [`Input::groups`].
    pub group: usize,
    /// The cells its label takes in full; a chip shows fewer, on up to [`ROWS`] rows, where there is less room.
    pub columns: usize,
    /// How long ago it was last active, in seconds, when known; an item whose age is not known is among the oldest.
    pub age: Option<u64>,
}

/// How far the layout reaches from its centre: to either side, above and below, in degrees. It fills the ellipse these
/// make, a half-ellipse above and another below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub side: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Limits {
    /// Whether `point` is within the limits.
    pub fn contains(&self, point: Vec2) -> bool {
        let vertical = if point.y >= 0.0 {
            self.top
        } else {
            self.bottom
        };
        (point.x / self.side).powi(2) + (point.y / vertical).powi(2) <= 1.0
    }

    /// How far from the centre the limit is in `direction`, a unit vector.
    pub fn reach(&self, direction: Vec2) -> f32 {
        let vertical = if direction.y >= 0.0 {
            self.top
        } else {
            self.bottom
        };
        1.0 / ((direction.x / self.side).powi(2) + (direction.y / vertical).powi(2)).sqrt()
    }

    /// The furthest the limits reach in any direction.
    fn furthest(&self) -> f32 {
        self.side.max(self.top).max(self.bottom)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub limits: Limits,
    /// The most cells a row of a chip's label shows near the centre, and the fewest it is cut to at the side limits.
    pub most_columns: usize,
    pub fewest_columns: usize,
    /// The most cells a sector's title shows; it is cut to fit, but never below [`Self::fewest_columns`].
    pub title_columns: usize,
    /// The most sectors shown: beyond them, the least recently active groups are merged into the last, "Other".
    pub most_sectors: usize,
    /// The least angle a sector is given, in degrees, so that its title and a few items fit.
    pub least_angle: f32,
}

impl Default for Options {
    /// As the headset shows sessions: within 40° to either side, 24° above and 30° below, two rows of up to 16 cells
    /// near the centre and 12 at the sides, and at most 8 sectors of at least 30°.
    fn default() -> Self {
        Self {
            limits: Limits {
                side: 40.0,
                top: 24.0,
                bottom: 30.0,
            },
            most_columns: 16,
            fewest_columns: 12,
            title_columns: 20,
            most_sectors: 8,
            least_angle: 30.0,
        }
    }
}

/// What to lay out: the groups' titles' widths, the items in order, and what stands at the centre, which is kept clear.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// The cells each group's title takes.
    pub groups: Vec<usize>,
    pub items: Vec<Item>,
    pub hub: Rect,
    /// The cells of the title of the sector of merged groups, such as "Other (7 folders)".
    pub other_columns: usize,
    /// The cells of the label of the chip that stands for a sector's items that do not fit, such as "12 older".
    pub more_columns: usize,
    /// The cells of the words that name a ring of age, such as "this week".
    pub note_columns: usize,
}

/// A chip placed: its item, its rectangle, and the cells each row of its label shows and how many rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    /// The item, by its index in [`Input::items`].
    pub item: usize,
    pub rect: Rect,
    pub columns: usize,
    pub rows: usize,
}

/// A sector of the circle: the groups it holds, one unless it is "Other", and what stands in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sector {
    pub groups: Vec<usize>,
    /// Its edges, in degrees clockwise from straight up; `end` is after `start`, and the sectors together make a turn
    /// from 0°.
    pub start: f32,
    pub end: f32,
    /// Its title's rectangle and the cells it shows, unless there is no room for it.
    pub title: Option<(Rect, usize)>,
    /// Its items shown, in their order, each no nearer the centre than the one before.
    pub chips: Vec<Chip>,
    /// How many of its items do not fit, always its oldest, and the chip that says so, if there is room for it.
    pub hidden: usize,
    pub more: Option<Rect>,
    /// How far the border along `start` reaches from the centre: from just clear of the centre to the limits.
    pub border: (f32, f32),
}

/// A guide between rings of age: every item inside it was active less than `age` seconds ago, every item outside it
/// longer ago.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub radius: f32,
    pub age: u64,
    /// Where the words naming the ring stand, just outside it, unless there is no room.
    pub note: Option<Rect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub sectors: Vec<Sector>,
    pub bands: Vec<Band>,
}

/// The unit vector `angle` degrees clockwise from straight up.
pub fn direction(angle: f32) -> Vec2 {
    let (sin, cos) = angle.to_radians().sin_cos();
    Vec2::new(sin, cos)
}

/// The angle of `point`, in degrees clockwise from straight up, from 0 up to a turn.
pub fn angle_of(point: Vec2) -> f32 {
    point.x.atan2(point.y).to_degrees().rem_euclid(360.0)
}

/// `rect` grown by `by` on every side.
pub fn grown(rect: Rect, by: f32) -> Rect {
    Rect {
        min: rect.min - Vec2::splat(by),
        max: rect.max + Vec2::splat(by),
    }
}

/// Whether `a` and `b` share any area.
pub fn overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// How far the nearest point of `rect` is from the centre.
pub fn nearest(rect: Rect) -> f32 {
    Vec2::ZERO.clamp(rect.min, rect.max).length()
}

/// The corners of `rect`.
pub fn corners(rect: Rect) -> [Vec2; 4] {
    [
        rect.min,
        Vec2::new(rect.max.x, rect.min.y),
        rect.max,
        Vec2::new(rect.min.x, rect.max.y),
    ]
}

/// Whether the half-line from the centre along `direction` passes through `rect`.
pub fn crosses(rect: Rect, direction: Vec2) -> bool {
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..2 {
        let (min, max, along) = (rect.min[axis], rect.max[axis], direction[axis]);
        if along.abs() < 1e-6 {
            if 0.0 < min || 0.0 > max {
                return false;
            }
            continue;
        }
        let (a, b) = (min / along, max / along);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

/// A rectangle `size` whose corner nearest the centre is at `corner`, extending away from the centre.
fn hung(corner: Vec2, size: Vec2) -> Rect {
    let x = if corner.x >= 0.0 {
        (corner.x, corner.x + size.x)
    } else {
        (corner.x - size.x, corner.x)
    };
    let y = if corner.y >= 0.0 {
        (corner.y, corner.y + size.y)
    } else {
        (corner.y - size.y, corner.y)
    };
    Rect {
        min: Vec2::new(x.0, y.0),
        max: Vec2::new(x.1, y.1),
    }
}

/// A rectangle `size` centred on `centre`.
fn centred(centre: Vec2, size: Vec2) -> Rect {
    Rect {
        min: centre - size / 2.0,
        max: centre + size / 2.0,
    }
}

/// The room left as chips are placed: what is taken, within the limits.
#[derive(Clone)]
struct Room<'a> {
    limits: &'a Limits,
    taken: Vec<Rect>,
}

impl Room<'_> {
    /// Whether `rect` is free and within the limits, and within the sector from `start` to `end` and clear of its
    /// edges, a whole turn having none.
    fn free(&self, rect: Rect, start: f32, end: f32) -> bool {
        if !corners(rect)
            .iter()
            .all(|corner| self.limits.contains(*corner))
        {
            return false;
        }
        let clear = grown(rect, GAP / 2.0);
        if end - start < 360.0 - 1e-3
            && ((angle_of(rect.centre()) - start).rem_euclid(360.0) > end - start
                || crosses(clear, direction(start))
                || crosses(clear, direction(end)))
        {
            return false;
        }
        !self
            .taken
            .iter()
            .any(|taken| overlap(clear, grown(*taken, GAP / 2.0)))
    }
}

/// The cells a row of a chip's label shows at `radius` from the centre: the most near it, fewer towards the sides.
fn columns_at(options: &Options, radius: f32) -> usize {
    let along = (radius / options.limits.side).clamp(0.0, 1.0);
    let most = options.most_columns as f32;
    let fewest = options.fewest_columns as f32;
    (most - (most - fewest) * along).round() as usize
}

/// The groups as sectors, the most recently active first, merging those beyond [`Options::most_sectors`] into the last.
fn grouped(input: &Input, options: &Options) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = Vec::new();
    for item in &input.items {
        if !order.contains(&item.group) {
            order.push(item.group);
        }
    }
    let most = options.most_sectors.max(1);
    if order.len() <= most {
        return order.into_iter().map(|group| vec![group]).collect();
    }
    let mut sectors: Vec<Vec<usize>> = order[..most - 1].iter().map(|group| vec![*group]).collect();
    sectors.push(order[most - 1..].to_vec());
    sectors
}

/// The ring of age that `age` falls in: 0 for today, up to [`BANDS`]'s length for older or unknown.
pub fn band(age: Option<u64>) -> usize {
    age.map_or(BANDS.len(), |age| {
        BANDS.iter().take_while(|limit| age >= **limit).count()
    })
}

/// Lay out `input`. See the module's documentation.
pub fn layout(input: &Input, options: &Options) -> Layout {
    let grouped = grouped(input, options);
    if grouped.is_empty() {
        return Layout::default();
    }
    let counts: Vec<usize> = grouped
        .iter()
        .map(|groups| {
            input
                .items
                .iter()
                .filter(|item| groups.contains(&item.group))
                .count()
        })
        .collect();
    let total: usize = counts.iter().sum();
    // Every sector has the least angle, and the rest is shared in proportion to how many items each has.
    let least = options.least_angle.min(360.0 / grouped.len() as f32);
    let spare = 360.0 - least * grouped.len() as f32;
    let mut room = Room {
        limits: &options.limits,
        taken: vec![input.hub],
    };
    let last = grouped.len() - 1;
    let mut start = 0.0;
    let mut sectors: Vec<Sector> = Vec::new();
    for (index, (groups, count)) in grouped.into_iter().zip(counts).enumerate() {
        // The last ends exactly a turn round, so that rounding leaves no gap.
        let end = if index == last {
            360.0
        } else {
            start + least + spare * count as f32 / total as f32
        };
        let columns = if groups.len() > 1 {
            input.other_columns
        } else {
            input.groups[groups[0]]
        };
        let title = title(options, &room, columns, start, end);
        if let Some((rect, _)) = &title {
            room.taken.push(*rect);
        }
        sectors.push(Sector {
            groups,
            start,
            end,
            title,
            chips: Vec::new(),
            hidden: 0,
            more: None,
            border: (
                border_start(input.hub, direction(start)),
                options.limits.reach(direction(start)),
            ),
        });
        start = end;
    }
    // The first ring stands clear of the centre all round, so that it circles it; each ring of age then starts outside
    // every item of the rings inside it, in every sector.
    let mut from = corners(input.hub)
        .iter()
        .map(|corner| corner.length())
        .fold(0.0, f32::max)
        + GAP;
    let mut stopped = vec![false; sectors.len()];
    let mut bands = Vec::new();
    for (ring, limit) in BANDS.iter().map(Some).chain([None]).enumerate() {
        let mut reached = from;
        for (index, sector) in sectors.iter_mut().enumerate() {
            let queue: Vec<usize> = (0..input.items.len())
                .filter(|number| {
                    let item = &input.items[*number];
                    sector.groups.contains(&item.group) && band(item.age) == ring
                })
                .collect();
            if stopped[index] {
                sector.hidden += queue.len();
                continue;
            }
            // Room for each, nearest the centre first; then each item in its order takes the next, shrunk to its label,
            // so that none stands nearer the centre than the one before.
            let found = slots(&room, options, queue.len(), from, sector.start, sector.end);
            for (slot, number) in found.iter().zip(&queue) {
                let corner = Vec2::ZERO.clamp(slot.min, slot.max);
                let (size, columns, rows) = chip(
                    input.items[*number].columns,
                    columns_at(options, corner.length()),
                );
                let rect = hung(corner, size);
                room.taken.push(rect);
                reached = reached.max(nearest(rect) + size.y + GAP);
                sector.chips.push(Chip {
                    item: *number,
                    rect,
                    columns,
                    rows,
                });
            }
            // Once one does not fit, none older is shown either.
            if found.len() < queue.len() {
                stopped[index] = true;
                sector.hidden += queue.len() - found.len();
            }
        }
        // A ring is drawn only between items: where some are inside it and some outside.
        let beyond = input.items.iter().any(|item| band(item.age) > ring);
        if let Some(limit) = limit
            && reached > from
            && beyond
        {
            // Its name stands just outside it, up and to the left, or wherever round it there is room.
            let size = Vec2::new(title_width(input.note_columns), LINE);
            let note = (0..72)
                .map(|step| hung(direction(315.0 - step as f32 * 5.0) * (reached + GAP), size))
                .find(|rect| room.free(*rect, 0.0, 360.0));
            if let Some(rect) = note {
                room.taken.push(rect);
            }
            bands.push(Band {
                radius: reached,
                age: *limit,
                note,
            });
            from = reached + LINE;
        }
    }
    // A sector with items that do not fit ends with a chip saying how many, made room for by its newest-but-last if need
    // be.
    for sector in &mut sectors {
        if sector.hidden == 0 {
            continue;
        }
        loop {
            let least = sector.chips.last().map_or(0.0, |chip| nearest(chip.rect));
            let size = Vec2::new(chip_width(input.more_columns), LINE);
            if let Some(rect) = outermost(&room, size, least, sector.start, sector.end) {
                room.taken.push(rect);
                sector.more = Some(rect);
                break;
            }
            let Some(chip) = sector.chips.pop() else {
                break;
            };
            room.taken.retain(|taken| *taken != chip.rect);
            sector.hidden += 1;
        }
    }
    Layout { sectors, bands }
}

/// How far from the centre a border along `direction` starts: just clear of the centre's `hub`.
fn border_start(hub: Rect, direction: Vec2) -> f32 {
    let hub = grown(hub, GAP);
    let mut distance = 0.0;
    while hub.contains(direction * distance) {
        distance += 0.1;
    }
    distance
}

/// The free place for a chip of `size` furthest from the centre, but at least `least` from it, and nearest the middle
/// of the sector from `start` to `end` at that distance, so that it leads further out. `None` when there is none.
fn outermost(room: &Room, size: Vec2, least: f32, start: f32, end: f32) -> Option<Rect> {
    let middle = (start + end) / 2.0;
    let mut angles: Vec<f32> = (0..=((end - start) as usize))
        .map(|step| start + step as f32)
        .collect();
    angles.sort_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()));
    let mut radius = room.limits.furthest();
    while radius >= least {
        if let Some(rect) = angles
            .iter()
            .map(|angle| hung(direction(*angle) * radius, size))
            .find(|rect| room.free(*rect, start, end))
        {
            return Some(rect);
        }
        radius -= RADIAL_STEP * 2.0;
    }
    None
}

/// Up to `count` places for chips of the largest size each distance allows, at least `least` from the centre, in the
/// sector from `start` to `end`: the nearest first, each ring clockwise from the sector's start. Where a ring has more
/// room than is still needed, those it holds are spread across the sector.
fn slots(
    room: &Room,
    options: &Options,
    count: usize,
    least: f32,
    start: f32,
    end: f32,
) -> Vec<Rect> {
    let mut taken = room.clone();
    let mut found: Vec<(f32, f32, Rect)> = Vec::new();
    let mut radius = least;
    while radius <= room.limits.furthest() && found.len() < count {
        let (size, _, _) = chip(usize::MAX, columns_at(options, radius));
        let steps = ((end - start).to_radians() * radius.max(1.0) / ARC_STEP).ceil() as usize;
        let angle = |step: usize| start + (end - start) * step as f32 / steps as f32;
        let at = |step: usize| hung(direction(angle(step)) * radius, size);
        // Every place on this ring, packed clockwise.
        let mut packed = taken.clone();
        let mut ring: Vec<usize> = Vec::new();
        for step in 0..=steps {
            if packed.free(at(step), start, end) {
                packed.taken.push(at(step));
                ring.push(step);
            }
        }
        // Where it has more room than is still needed, every so many of its places, spread across the sector.
        let need = count - found.len();
        if ring.len() > need {
            ring = (0..need)
                .map(|index| ring[index * ring.len() / need + ring.len() / (2 * need)])
                .collect();
        }
        for step in ring {
            taken.taken.push(at(step));
            found.push((radius, angle(step), at(step)));
        }
        radius += RADIAL_STEP;
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    found.into_iter().map(|(_, _, rect)| rect).collect()
}

/// A sector's title: centred on the sector's middle, as far out as it fits, cut to fit, but not below the fewest cells a
/// chip shows.
fn title(
    options: &Options,
    room: &Room,
    wanted: usize,
    start: f32,
    end: f32,
) -> Option<(Rect, usize)> {
    let wanted = wanted.min(options.title_columns).max(1);
    let middle = direction((start + end) / 2.0);
    let fewest = options.fewest_columns.min(wanted);
    for columns in (fewest..=wanted).rev() {
        let size = Vec2::new(title_width(columns), LINE);
        let mut distance = options.limits.reach(middle);
        while distance >= 0.0 {
            let rect = centred(middle * distance, size);
            if room.free(rect, start, end) {
                return Some((rect, columns));
            }
            distance -= RADIAL_STEP;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Rect {
        centred(Vec2::ZERO, Vec2::new(chip_width(16), LINE * 4.0))
    }

    /// `counts` items in each group, interleaved as work moves between them, the newest first, each an hour older than
    /// the last, so that they span every ring of age.
    fn input(counts: &[usize], hours: u64) -> Input {
        let mut items = Vec::new();
        let mut left = counts.to_vec();
        while left.iter().any(|left| *left > 0) {
            for (group, left) in left.iter_mut().enumerate() {
                if *left > 0 {
                    *left -= 1;
                    let n = items.len();
                    items.push(Item {
                        group,
                        columns: 10 + (n * 7) % 30,
                        age: Some(n as u64 * hours * 3_600),
                    });
                }
            }
        }
        Input {
            groups: counts.iter().map(|_| 16).collect(),
            items,
            hub: hub(),
            other_columns: 17,
            more_columns: 9,
            note_columns: 10,
        }
    }

    /// Every rectangle laid out: the hub's, the titles', the chips', the "older" chips' and the notes'.
    fn rectangles(input: &Input, layout: &Layout) -> Vec<Rect> {
        let mut rects = vec![input.hub];
        for sector in &layout.sectors {
            rects.extend(sector.chips.iter().map(|chip| chip.rect));
            rects.extend(sector.title.iter().map(|(rect, _)| *rect));
            rects.extend(sector.more);
        }
        rects.extend(layout.bands.iter().filter_map(|band| band.note));
        rects
    }

    const SCALES: [&[usize]; 5] = [
        &[10],
        &[7, 3],
        &[16, 9, 5],
        &[38, 17, 11, 7, 4, 3],
        &[70, 32, 22, 16, 13, 10, 8, 7, 6, 5, 4, 3, 2, 1, 1],
    ];

    #[test]
    fn sectors_partition_the_circle_and_nothing_overlaps_or_leaves_the_limits() {
        let options = Options::default();
        for counts in SCALES {
            let input = input(counts, 5);
            let layout = layout(&input, &options);
            let mut start = 0.0;
            for sector in &layout.sectors {
                assert_eq!(sector.start, start);
                assert!(sector.end - sector.start >= options.least_angle - 1e-3);
                start = sector.end;
            }
            assert_eq!(start, 360.0, "{counts:?}");
            let rects = rectangles(&input, &layout);
            for (index, a) in rects.iter().enumerate() {
                for b in &rects[index + 1..] {
                    assert!(!overlap(*a, *b), "{counts:?}: {a:?} and {b:?} overlap");
                }
                for corner in corners(*a) {
                    assert!(options.limits.contains(corner), "{counts:?}: {a:?}");
                }
            }
            // Each sector's chips and title stand within it, clear of its borders.
            for sector in layout.sectors.iter().filter(|_| layout.sectors.len() > 1) {
                let inside = sector
                    .chips
                    .iter()
                    .map(|chip| chip.rect)
                    .chain(sector.title.map(|(rect, _)| rect))
                    .chain(sector.more);
                for rect in inside {
                    let angle = (angle_of(rect.centre()) - sector.start).rem_euclid(360.0);
                    assert!(angle <= sector.end - sector.start, "{counts:?}");
                    for edge in [sector.start, sector.end] {
                        assert!(!crosses(rect, direction(edge)), "{counts:?}: {rect:?}");
                    }
                }
                assert!(sector.border.0 < sector.border.1);
                assert!(
                    !input
                        .hub
                        .contains(direction(sector.start) * sector.border.0)
                );
            }
        }
    }

    #[test]
    fn labels_are_legible_and_shorter_only_towards_the_sides() {
        // About half a degree of x-height: a font pixel of at least 0.08°, as `design/text.md` asks.
        const { assert!(PIXEL * 5.7 >= 0.45) };
        let options = Options::default();
        for counts in SCALES {
            let input = input(counts, 5);
            for sector in &layout(&input, &options).sectors {
                for chip in &sector.chips {
                    assert!(
                        chip.columns >= options.fewest_columns.min(input.items[chip.item].columns)
                    );
                    assert!(chip.columns <= options.most_columns);
                    assert!(
                        (chip.rect.max.y - chip.rect.min.y - chip_height(chip.rows)).abs() < 1e-4
                    );
                    // A label that does not fit a row wraps onto a second rather than being cut to one.
                    let full = input.items[chip.item].columns;
                    assert_eq!(chip.rows, if full <= chip.columns { 1 } else { 2 });
                }
                assert!(
                    sector
                        .title
                        .is_none_or(|(_, columns)| columns >= options.fewest_columns)
                );
            }
        }
        assert_eq!(columns_at(&options, 0.0), 16);
        assert_eq!(columns_at(&options, 80.0), 12);
        assert_eq!(chip(0, 16).1, 1);
    }

    #[test]
    fn items_stand_further_out_the_older_they_are_with_rings_of_age_between() {
        for counts in SCALES {
            let input = input(counts, 5);
            let layout = layout(&input, &Options::default());
            for sector in &layout.sectors {
                // In order, and each no nearer the centre than the one before.
                let items: Vec<usize> = sector.chips.iter().map(|chip| chip.item).collect();
                let mut sorted = items.clone();
                sorted.sort();
                assert_eq!(items, sorted);
                for pair in sector.chips.windows(2) {
                    assert!(
                        nearest(pair[1].rect) >= nearest(pair[0].rect) - 1e-4,
                        "{counts:?}"
                    );
                }
                // What does not fit is the sector's oldest.
                let all: Vec<usize> = (0..input.items.len())
                    .filter(|n| sector.groups.contains(&input.items[*n].group))
                    .collect();
                assert_eq!(items, all[..items.len()], "{counts:?}");
                assert_eq!(items.len() + sector.hidden, all.len());
            }
            // Every item inside a ring was active more recently than it says, and every one outside it longer ago.
            for band in &layout.bands {
                for chip in layout.sectors.iter().flat_map(|sector| &sector.chips) {
                    let age = input.items[chip.item].age.unwrap();
                    assert_eq!(
                        nearest(chip.rect) < band.radius,
                        age < band.age,
                        "{counts:?}"
                    );
                }
            }
        }
        // Ten sessions an hour apart are all today: no ring is drawn.
        assert!(
            layout(&input(&[6, 4], 1), &Options::default())
                .bands
                .is_empty()
        );
        // Three days apart, there are rings for today, this week and this month, each named.
        let days = layout(&input(&[6, 4, 3], 72), &Options::default());
        let ages: Vec<u64> = days.bands.iter().map(|band| band.age).collect();
        assert_eq!(ages, BANDS);
        assert!(days.bands.iter().all(|band| band.note.is_some()));
        assert!(
            days.bands
                .windows(2)
                .all(|pair| pair[0].radius < pair[1].radius)
        );
    }

    #[test]
    fn items_whose_age_is_not_known_stand_with_the_oldest() {
        assert_eq!(band(None), 3);
        assert_eq!(band(Some(0)), 0);
        assert_eq!(band(Some(86_400)), 1);
        assert_eq!(band(Some(40 * 86_400)), 3);
        let mut spaces = input(&[3, 2], 1);
        for item in &mut spaces.items {
            item.age = None;
        }
        let layout = layout(&spaces, &Options::default());
        assert!(layout.bands.is_empty());
        assert_eq!(
            layout
                .sectors
                .iter()
                .map(|sector| sector.chips.len())
                .sum::<usize>(),
            5
        );
    }

    #[test]
    fn sectors_share_the_circle_by_how_many_items_each_has() {
        let layout = layout(&input(&[30, 10, 5, 5], 5), &Options::default());
        let angles: Vec<f32> = layout
            .sectors
            .iter()
            .map(|sector| sector.end - sector.start)
            .collect();
        // Each has 30°, and the 240° left is shared 30:10:5:5.
        for (angle, expected) in angles.iter().zip([174.0, 78.0, 54.0, 54.0]) {
            assert!((angle - expected).abs() < 1e-3, "{angles:?}");
        }
        // The most recently active group comes first, from straight up, clockwise.
        assert_eq!(layout.sectors[0].groups, [0]);
        // So many sectors that the least angle would overrun a turn share it equally.
        let options = Options {
            most_sectors: 20,
            ..Options::default()
        };
        let crowded = super::layout(&input(&[1; 15], 1), &options);
        for sector in &crowded.sectors {
            assert!((sector.end - sector.start - 24.0).abs() < 1e-3);
        }
    }

    #[test]
    fn groups_beyond_the_most_sectors_are_merged_into_the_last() {
        let counts = &[70, 32, 22, 16, 13, 10, 8, 7, 6, 5, 4, 3, 2, 1, 1];
        let input = input(counts, 5);
        let layout = layout(&input, &Options::default());
        assert_eq!(layout.sectors.len(), 8);
        for (index, sector) in layout.sectors[..7].iter().enumerate() {
            assert_eq!(sector.groups, [index]);
        }
        assert_eq!(layout.sectors[7].groups, (7..15).collect::<Vec<_>>());
        // Its title is "Other"'s, of its own width.
        let (rect, columns) = layout.sectors[7].title.unwrap();
        assert_eq!(columns, 17);
        assert!((rect.max.x - rect.min.x - title_width(17)).abs() < 1e-4);
        // Exactly as many as fit stand alone.
        let eight = super::layout(&super::tests::input(&counts[..8], 5), &Options::default());
        assert!(eight.sectors.iter().all(|sector| sector.groups.len() == 1));
    }

    #[test]
    fn a_sector_whose_items_do_not_fit_says_how_many_further_out() {
        let input = input(&[150, 50], 2);
        let layout = layout(&input, &Options::default());
        for sector in &layout.sectors {
            assert!(sector.hidden > 0);
            let more = sector.more.unwrap();
            let last = sector.chips.last().unwrap();
            assert!(nearest(more) >= nearest(last.rect));
        }
        // With no room even for that, its chips give way to it, and with none left it goes without.
        let options = Options {
            limits: Limits {
                side: 12.0,
                top: 6.0,
                bottom: 6.0,
            },
            ..Options::default()
        };
        let small = Input {
            hub: centred(Vec2::ZERO, Vec2::splat(2.0)),
            ..super::tests::input(&[40], 1)
        };
        let cramped = super::layout(&small, &options);
        let sector = &cramped.sectors[0];
        assert_eq!(sector.chips.len() + sector.hidden, 40);
        assert!(sector.more.is_some());
        let none = Options {
            limits: Limits {
                side: 3.0,
                top: 3.0,
                bottom: 3.0,
            },
            ..options
        };
        let nothing = super::layout(&small, &none);
        assert_eq!(nothing.sectors[0].chips.len(), 0);
        assert_eq!(nothing.sectors[0].hidden, 40);
        assert_eq!(nothing.sectors[0].more, None);
        assert_eq!(nothing.sectors[0].title, None);
    }

    #[test]
    fn one_group_fills_the_circle_and_nothing_lays_out_nothing() {
        let input = input(&[12], 5);
        let layout = layout(&input, &Options::default());
        assert_eq!(layout.sectors.len(), 1);
        assert_eq!(
            (layout.sectors[0].start, layout.sectors[0].end),
            (0.0, 360.0)
        );
        assert_eq!(layout.sectors[0].chips.len(), 12);
        // Its items go all round the centre, on both sides of it.
        let chips = &layout.sectors[0].chips;
        assert!(chips.iter().any(|chip| chip.rect.max.x < 0.0));
        assert!(chips.iter().any(|chip| chip.rect.min.x > 0.0));
        let empty = Input {
            items: Vec::new(),
            ..input
        };
        assert_eq!(
            super::layout(&empty, &Options::default()),
            Layout::default()
        );
    }

    #[test]
    fn a_ring_with_room_to_spare_spreads_its_items_across_the_sector() {
        let input = input(&[3, 1], 1);
        let layout = layout(&input, &Options::default());
        let first = &layout.sectors[0];
        let angles: Vec<f32> = first
            .chips
            .iter()
            .map(|chip| angle_of(Vec2::ZERO.clamp(chip.rect.min, chip.rect.max)))
            .collect();
        // Three items in a sector of over 200° are not packed at its start.
        assert!(
            angles.iter().cloned().fold(0.0, f32::max) - angles[0] > 60.0,
            "{angles:?}"
        );
    }

    #[test]
    fn geometry_helpers_agree() {
        let limits = Options::default().limits;
        assert!((limits.reach(Vec2::X) - 40.0).abs() < 1e-4);
        assert!((limits.reach(Vec2::Y) - 24.0).abs() < 1e-4);
        assert!((limits.reach(-Vec2::Y) - 30.0).abs() < 1e-4);
        assert!(limits.contains(Vec2::new(0.0, -29.0)) && !limits.contains(Vec2::new(0.0, 25.0)));
        assert_eq!(limits.furthest(), 40.0);
        assert!((angle_of(Vec2::new(-1.0, 0.0)) - 270.0).abs() < 1e-4);
        let square = Rect {
            min: Vec2::new(1.0, -1.0),
            max: Vec2::new(3.0, 1.0),
        };
        assert!(crosses(square, Vec2::X));
        assert!(!crosses(square, -Vec2::X));
        assert!(!crosses(square, Vec2::Y));
        assert!(crosses(grown(square, 1.5), Vec2::Y));
        assert_eq!(nearest(square), 1.0);
        let below_left = hung(Vec2::new(-1.0, -1.0), Vec2::new(2.0, 1.0));
        assert_eq!(below_left.max, Vec2::new(-1.0, -1.0));
        assert_eq!(nearest(below_left), 2f32.sqrt());
    }
}
