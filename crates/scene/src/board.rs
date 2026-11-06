// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The GitHub environment's cards in a space, a prototype (issue #148): where each card stands, in the inbox or where a
//! participant put it, what its face shows, which card the ray points at, and picking one up with the grip, carrying it
//! along the ray and putting it down. See `design/github.md`.
//!
//! Where cards are put is the space's, shared by everyone in it, so a card put down is sent to the server, and shown
//! where it was put until the server's cards say so too. Nothing here reaches GitHub.
use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec3};
use tomori_protocol::{Card, CardKind, Cards, Checks, Pose};

use crate::{
    arrival::HOME,
    canvas::Canvas,
    controller_ui::Sheet,
    pointing::Ray,
    recording::facing,
    ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Span, Style, Wrap, cut, wrap},
};

/// A card's face, in cells.
pub const COLUMNS: usize = 18;
pub const ROWS: usize = 6;
/// The size of a font pixel on a card, in metres: a card is about 14 by 6 cm.
pub const PIXEL: f32 = 0.0008;
const PADDING: (usize, usize) = (4, 3);

/// Where the inbox stands, front left of home at about waist height, and the point it faces, which tilts it back like
/// a lectern.
pub const INBOX: Vec3 = Vec3::new(-0.42, 1.0, -0.5);
/// The most cards in one column of the inbox; more start another column.
pub const PER_COLUMN: usize = 12;
/// The gap between the inbox's columns, and around them.
const GAP: f32 = 0.012;
/// How much nearer each card in a column is than the one above it, so that it covers it, except for its first two
/// rows.
const LIFT: f32 = 0.0015;
/// How far a released card may be from the inbox, beyond its edges or off its face, and still go back into it.
const INBOX_MARGIN: f32 = 0.05;
const INBOX_DEPTH: f32 = 0.15;

/// How near and far a card can be carried along the ray, in metres.
const NEAREST: f32 = 0.2;
const FARTHEST: f32 = 4.0;

/// How long a card put down is shown where it was put, while the server's cards do not yet say so.
pub const PENDING: Duration = Duration::from_secs(3);

/// How firmly the grip is squeezed to pick up a card, and how far it is let go to put it down.
const SQUEEZE: f32 = 0.75;
const RELEASE: f32 = 0.5;

const WHITE: [f32; 4] = [0.92, 0.95, 1.0, 1.0];
const DIM: [f32; 4] = [0.6, 0.66, 0.74, 1.0];
const GREEN: [f32; 4] = [0.25, 0.85, 0.4, 1.0];
const RED: [f32; 4] = [1.0, 0.33, 0.3, 1.0];
const AMBER: [f32; 4] = [1.0, 0.72, 0.2, 1.0];
const GREY: [f32; 4] = [0.45, 0.5, 0.56, 1.0];
const HIGHLIGHT: [f32; 4] = [0.35, 0.95, 1.0, 1.0];

/// The size of a card, in metres.
pub fn card_size() -> (f32, f32) {
    let width = COLUMNS * CELL_WIDTH + 2 * PADDING.0;
    let height = ROWS * CELL_HEIGHT + 2 * PADDING.1;
    (width as f32 * PIXEL, height as f32 * PIXEL)
}

/// How much of a card in the inbox shows above the next one: its first two rows.
fn peek() -> f32 {
    (PADDING.1 + 2 * CELL_HEIGHT) as f32 * PIXEL
}

/// The inbox's own frame in the scene: at [`INBOX`], facing home.
pub fn inbox_frame() -> Mat4 {
    facing(INBOX, HOME, 1.0)
}

/// The height of the inbox's label row.
fn label_height() -> f32 {
    (CELL_HEIGHT + 2 * PADDING.1) as f32 * PIXEL
}

/// The inbox's size, in metres, holding `count` cards.
fn inbox_size(count: usize) -> (f32, f32) {
    let (width, height) = card_size();
    let columns = count.div_ceil(PER_COLUMN).max(1);
    let rows = count.clamp(1, PER_COLUMN);
    (
        columns as f32 * (width + GAP) + GAP,
        label_height() + GAP + (rows - 1) as f32 * peek() + height + GAP,
    )
}

/// The model of the `index`th of `count` cards in the inbox, in the inbox's frame.
fn inbox_slot(index: usize, count: usize) -> Mat4 {
    let (width, height) = card_size();
    let (_, tray) = inbox_size(count);
    let columns = count.div_ceil(PER_COLUMN).max(1);
    let (column, row) = (index / PER_COLUMN, index % PER_COLUMN);
    let x = (column as f32 - (columns - 1) as f32 / 2.0) * (width + GAP);
    let y = tray / 2.0 - label_height() - GAP - row as f32 * peek() - height / 2.0;
    Mat4::from_translation(Vec3::new(x, y, 0.004 + row as f32 * LIFT))
}

/// A pose as a model.
fn model(pose: &Pose) -> Option<Mat4> {
    let rotation = Quat::from_array(pose.orientation);
    let position = Vec3::from_array(pose.position);
    (rotation.is_finite() && position.is_finite() && rotation.length() > 0.5)
        .then(|| Mat4::from_rotation_translation(rotation.normalize(), position))
}

/// A model as a pose, without its scale.
fn pose(model: Mat4) -> Pose {
    let (_, rotation, position) = model.to_scale_rotation_translation();
    Pose {
        position: position.to_array(),
        orientation: rotation.normalize().to_array(),
    }
}

/// Where `ray` meets a card drawn with `model`, as the distance along it, if it does.
fn hit(model: Mat4, ray: &Ray) -> Option<f32> {
    let (width, height) = card_size();
    let local = model.inverse();
    let origin = local.transform_point3(ray.origin);
    let direction = local.transform_vector3(ray.direction);
    if direction.z.abs() < f32::EPSILON {
        return None;
    }
    let along = -origin.z / direction.z;
    let point = origin + direction * along;
    (along >= 0.0 && point.x.abs() <= width / 2.0 && point.y.abs() <= height / 2.0).then_some(along)
}

/// A colour of six hexadecimal digits, lightened so that it reads on a card's dark face.
fn label_color(hex: &str) -> [f32; 4] {
    let channel = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|digits| u8::from_str_radix(digits, 16).ok())
            .map_or(0.6, |value| 0.35 + 0.65 * f32::from(value) / 255.0)
    };
    [channel(0), channel(2), channel(4), 1.0]
}

/// How long ago, briefly, such as `5m`, `3h` or `2d`.
pub fn age(seconds: u64) -> String {
    match seconds {
        0..3_600 => format!("{}m", seconds / 60),
        3_600..172_800 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// What a card's face shows: its number, kind and checks; its title on up to three rows; who opened it and how long
/// ago; and its labels. A card `covered` in the inbox shows only its first two rows, which is all of it that shows, so
/// that it costs little to draw. A `highlighted` card, pointed at or held, is framed brightly.
pub fn face(card: &Card, covered: bool, highlighted: bool) -> Canvas {
    let mut canvas = Canvas::new(COLUMNS, PIXEL);
    canvas.padding = PADDING;
    canvas.background = match card.kind {
        CardKind::PullRequest => [0.09, 0.06, 0.14, 1.0],
        CardKind::Draft => [0.08, 0.08, 0.09, 1.0],
        CardKind::Issue | CardKind::Unknown => [0.04, 0.08, 0.12, 1.0],
    };
    let (checks, color) = match card.checks {
        Checks::Passing => ("PASS", GREEN),
        Checks::Failing => ("FAIL", RED),
        Checks::Pending => ("WAIT", AMBER),
        Checks::None | Checks::Unknown => ("", GREY),
    };
    canvas.border = Some(if highlighted { HIGHLIGHT } else { color });
    let kind = match card.kind {
        CardKind::Issue => "issue",
        CardKind::PullRequest => "PR",
        CardKind::Draft => "draft",
        CardKind::Unknown => "",
    };
    let number = format!("#{}", card.number);
    let gap = COLUMNS.saturating_sub(number.len() + 1 + kind.len() + checks.len());
    let header = [
        Span::new(number, Style::bold(WHITE)),
        Span::new(format!(" {kind}"), Style::plain(DIM)),
        Span::new(" ".repeat(gap), Style::plain(DIM)),
        Span::new(checks, Style::bold(color)),
    ];
    let mut rows = vec![Row::new(cut(&header, COLUMNS))];
    let mut title = wrap(
        &[Span::new(&card.title, Style::plain(WHITE))],
        COLUMNS,
        Wrap::default(),
    );
    // A longer title ends its third row with "…".
    if title.len() > 3 {
        title.truncate(3);
        let last = &mut title[2];
        last.truncate(COLUMNS - 1);
        last.push(('…', Style::plain(WHITE)));
    }
    title.resize(3, Vec::new());
    rows.extend(title.into_iter().map(Row::new));
    rows.push(Row::text(
        &format!("@{} {}", card.author, age(card.age)),
        Style::plain(DIM),
        COLUMNS,
    ));
    let labels: Vec<Span> = card
        .labels
        .iter()
        .flat_map(|label| {
            [
                Span::new(&label.name, Style::plain(label_color(&label.color))),
                Span::new(" ", Style::plain(DIM)),
            ]
        })
        .collect();
    rows.push(Row::new(cut(&labels, COLUMNS)));
    if covered {
        for row in &mut rows[2..] {
            *row = Row::default();
        }
    }
    canvas.rows = rows;
    canvas
}

/// The inbox's own face, holding `count` cards from `source`, with why they may be missing or out of date.
fn tray(source: &str, count: usize, status: Option<&str>) -> Canvas {
    let (width, height) = inbox_size(count);
    let columns = ((width / PIXEL) as usize).saturating_sub(2 * PADDING.0) / CELL_WIDTH;
    let rows = (((height / PIXEL) as usize).saturating_sub(2 * PADDING.1) / CELL_HEIGHT).max(1);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.03, 0.05, 0.92];
    canvas.border = Some([0.3, 0.4, 0.5, 0.8]);
    let label = match status {
        Some(status) => format!("INBOX {source}: {status}"),
        None => format!("INBOX {source} ({count})"),
    };
    canvas.rows = vec![Row::text(&label, Style::bold(DIM), columns)];
    canvas.rows.resize(rows, Row::default());
    canvas
}

/// A grip's squeeze, with hysteresis, as presses and releases.
#[derive(Clone, Copy, Debug, Default)]
struct Grip {
    held: bool,
}

impl Grip {
    /// Whether `value` pressed or released the grip this frame.
    fn update(&mut self, value: Option<f32>) -> (bool, bool) {
        let value = value.filter(|value| value.is_finite()).unwrap_or(0.0);
        let held = if self.held {
            value > RELEASE
        } else {
            value > SQUEEZE
        };
        let edges = (held && !self.held, !held && self.held);
        self.held = held;
        edges
    }
}

/// The card being carried.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    card: String,
    /// How far along the ray it is carried.
    distance: f32,
    /// Where it was last drawn, which it stays at while the ray is not tracked.
    model: Mat4,
}

/// A card put down, shown there until the server's cards say so or [`PENDING`] passes.
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    card: String,
    placement: Option<Pose>,
    since: Instant,
}

/// What the board needs this frame.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    /// The space's cards, while they are shown.
    pub cards: Option<&'a Cards>,
    /// The pointing ray, while it can point at cards.
    pub ray: Option<Ray>,
    /// The right grip's value, while its action is active.
    pub grip: Option<f32>,
    /// The point a carried card faces.
    pub viewer: Option<Vec3>,
    /// Whether the ray and grip act on the cards: in a space, with nothing else taking them, such as the hand menu or
    /// the right wrist's transcript. Otherwise a card carried is put down where it is.
    pub active: bool,
    pub now: Instant,
}

/// What the board did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// The inbox, then each card, to draw.
    pub sheets: Vec<Sheet>,
    /// The card the ray points at, by identity, and how far along the ray and where it meets it.
    pub pointed: Option<(String, f32, Vec3)>,
    /// A card was picked up.
    pub picked: bool,
    /// A card was put down, where, or back in the inbox: to send to the server.
    pub placed: Option<(String, Option<Pose>)>,
    /// Where the ray ends on a card: where it meets the card pointed at, or the middle of the card carried.
    pub end: Option<Vec3>,
}

/// The cards across frames: the card carried, and the card put down that the server has not yet confirmed.
#[derive(Clone, Debug, Default)]
pub struct Board {
    grip: Grip,
    held: Option<Held>,
    pending: Option<Pending>,
}

impl Board {
    /// Follow `input`, and say what to draw and send.
    pub fn update(&mut self, input: Input<'_>) -> Output {
        // The grip is followed even while the cards do not take it, so that a squeeze held as they start to is not a new one.
        let (pressed, released) = self.grip.update(input.grip);
        let Some(cards) = input.cards else {
            self.held = None;
            self.pending = None;
            return Output::default();
        };
        // A card put down shows where it was put until the server agrees, or for a while if it does not.
        if let Some(pending) = &self.pending {
            let confirmed = cards
                .cards
                .iter()
                .find(|card| card.id == pending.card)
                .is_none_or(|card| card.placement == pending.placement);
            if confirmed || input.now.saturating_duration_since(pending.since) >= PENDING {
                self.pending = None;
            }
        }
        // A card that left the board, as when it closed on GitHub, is no longer carried.
        if let Some(held) = &self.held
            && !cards.cards.iter().any(|card| card.id == held.card)
        {
            self.held = None;
        }
        let frame = inbox_frame();
        let viewer = input.viewer.unwrap_or(HOME);
        let mut output = Output::default();
        // The card carried follows the ray, facing the viewer.
        if let Some(held) = &mut self.held
            && let Some(ray) = input.ray.filter(|_| input.active)
        {
            held.model = facing(ray.at(held.distance), viewer, 1.0);
        }
        // Let go, or taken by something else, it is put down where it is, or back in the inbox over it.
        if (released || !input.active)
            && let Some(held) = self.held.take()
        {
            let inside = frame
                .inverse()
                .transform_point3(held.model.w_axis.truncate());
            let (width, height) = inbox_size(self.inboxed(cards));
            let into_inbox = inside.x.abs() <= width / 2.0 + INBOX_MARGIN
                && inside.y.abs() <= height / 2.0 + INBOX_MARGIN
                && inside.z.abs() <= INBOX_DEPTH;
            let placement = (!into_inbox).then(|| pose(held.model));
            self.pending = Some(Pending {
                card: held.card.clone(),
                placement,
                since: input.now,
            });
            output.placed = Some((held.card, placement));
        }
        let inboxed = self.inboxed(cards);
        let mut slot = 0;
        let models: Vec<(Mat4, bool)> = cards
            .cards
            .iter()
            .map(|card| match self.stands(card) {
                Some(model) => (model, false),
                None => {
                    slot += 1;
                    // Only the last card of each column shows in full.
                    let covered = slot % PER_COLUMN != 0 && slot != inboxed;
                    (frame * inbox_slot(slot - 1, inboxed), covered)
                }
            })
            .collect();
        // The nearest card the ray meets, other than one carried, is pointed at.
        let carried = self.held();
        output.pointed = input.ray.filter(|_| input.active).and_then(|ray| {
            cards
                .cards
                .iter()
                .zip(&models)
                .filter(|(card, _)| Some(card.id.as_str()) != carried)
                .filter_map(|(card, (model, _))| {
                    hit(*model, &ray).map(|distance| (card.id.clone(), distance, ray.at(distance)))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
        });
        // A squeeze picks up the card pointed at.
        if pressed
            && carried.is_none()
            && let Some((card, distance, _)) = output.pointed.take()
        {
            let index = cards
                .cards
                .iter()
                .position(|known| known.id == card)
                .expect("pointed at a card on the board");
            self.held = Some(Held {
                card,
                distance: distance.clamp(NEAREST, FARTHEST),
                model: models[index].0,
            });
            self.pending = None;
            output.picked = true;
        }
        output.end = match &self.held {
            Some(held) => Some(held.model.w_axis.truncate()),
            None => output.pointed.as_ref().map(|(_, _, point)| *point),
        };
        let highlighted = self
            .held()
            .or(output.pointed.as_ref().map(|(card, _, _)| card.as_str()));
        output.sheets.push(Sheet {
            canvas: Some(tray(&cards.source, inboxed, cards.status.as_deref())),
            model: Some(frame),
        });
        // Panels do not write depth, so the cards are drawn farthest from the viewer first, and a nearer card covers
        // one behind it; the inbox, behind all of its own, comes first.
        let mut order: Vec<usize> = (0..models.len()).collect();
        let distance = |index: &usize| models[*index].0.w_axis.truncate().distance(viewer);
        order.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
        output.sheets.extend(order.into_iter().map(|index| {
            let (card, (model, covered)) = (&cards.cards[index], &models[index]);
            let highlighted = highlighted == Some(card.id.as_str());
            Sheet {
                canvas: Some(face(card, *covered && !highlighted, highlighted)),
                model: Some(*model),
            }
        }));
        output
    }

    /// Where `card` stands, when it is not in the inbox: carried, put down but not yet confirmed, or where the server
    /// says it was put.
    fn stands(&self, card: &Card) -> Option<Mat4> {
        match (&self.held, &self.pending) {
            (Some(held), _) if held.card == card.id => Some(held.model),
            (_, Some(pending)) if pending.card == card.id => {
                pending.placement.as_ref().and_then(model)
            }
            _ => card.placement.as_ref().and_then(model),
        }
    }

    /// How many of `cards` are in the inbox.
    fn inboxed(&self, cards: &Cards) -> usize {
        cards
            .cards
            .iter()
            .filter(|card| self.stands(card).is_none())
            .count()
    }

    /// The card being carried, by identity.
    pub fn held(&self) -> Option<&str> {
        self.held.as_ref().map(|held| held.card.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::Label;

    fn card(number: u64) -> Card {
        Card {
            id: format!("github://o/r/issues/{number}"),
            number,
            kind: CardKind::Issue,
            title: format!("Card {number}"),
            author: "ada".into(),
            labels: Vec::new(),
            checks: Checks::None,
            age: 7_200,
            placement: None,
        }
    }

    fn cards(count: u64) -> Cards {
        Cards {
            space: "s".into(),
            source: "o/r".into(),
            cards: (1..=count).map(card).collect(),
            status: None,
        }
    }

    fn input(cards: &Cards, ray: Option<Ray>, grip: f32, now: Instant) -> Input<'_> {
        Input {
            cards: Some(cards),
            ray,
            grip: Some(grip),
            viewer: Some(HOME),
            active: true,
            now,
        }
    }

    /// A ray from home towards the centre of the sheet drawn with `model`.
    fn at(model: Mat4) -> Ray {
        Ray::new(HOME, model.w_axis.truncate() - HOME).unwrap()
    }

    /// A ray from home towards the header of the card drawn with `model`, which shows when the next card in the inbox
    /// covers the rest.
    fn header(model: Mat4) -> Ray {
        at(model * Mat4::from_translation(Vec3::new(0.0, 0.02, 0.0)))
    }

    /// The sheet of the card numbered `number`.
    fn sheet(output: &Output, number: u64) -> &Sheet {
        let header = format!("#{number} ");
        output.sheets[1..]
            .iter()
            .find(|sheet| text(sheet.canvas.as_ref().unwrap())[0].starts_with(&header))
            .unwrap()
    }

    /// The models of the cards numbered from 1 to `count`, in order.
    fn models(output: &Output, count: u64) -> Vec<Mat4> {
        (1..=count)
            .map(|number| sheet(output, number).model.unwrap())
            .collect()
    }

    fn text(canvas: &Canvas) -> Vec<String> {
        canvas
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|(ch, _)| *ch).collect::<String>())
            .collect()
    }

    #[test]
    fn a_face_shows_the_number_kind_checks_title_author_age_and_labels() {
        let mut pull = card(164);
        pull.kind = CardKind::PullRequest;
        pull.checks = Checks::Failing;
        pull.title =
            "Make the client crates ready to publish on crates.io at last, with readmes".into();
        pull.labels = vec![
            Label {
                name: "bug".into(),
                color: "d73a4a".into(),
            },
            Label {
                name: "odd".into(),
                color: "zz".into(),
            },
        ];
        let canvas = face(&pull, false, false);
        let rows = text(&canvas);
        assert_eq!(rows.len(), ROWS);
        assert_eq!(rows[0], "#164 PR       FAIL");
        assert_eq!(rows[1], "Make the client");
        assert!(rows[3].ends_with('…'), "{rows:?}");
        assert!(rows[3].chars().count() <= COLUMNS);
        assert_eq!(rows[4], "@ada 2h");
        assert_eq!(rows[5], "bug odd ");
        assert_eq!(canvas.border, Some(RED));
        assert_eq!(canvas.size(), card_size());
        // The label keeps its colour, lightened; one that is not a colour is grey.
        assert_eq!(canvas.rows[5].cells[0].1.color, label_color("d73a4a"));
        assert_eq!(label_color("zz"), [0.6, 0.6, 0.6, 1.0]);
        assert_eq!(label_color("000000"), [0.35, 0.35, 0.35, 1.0]);
        // Highlighted, it is framed brightly; covered, it shows only its first two rows.
        assert_eq!(face(&pull, false, true).border, Some(HIGHLIGHT));
        let covered = text(&face(&pull, true, false));
        assert_eq!(&covered[..2], &rows[..2]);
        assert!(covered[2..].iter().all(String::is_empty));
        for (kind, checks, header, border) in [
            (CardKind::Issue, Checks::None, "#7 issue", GREY),
            (
                CardKind::Draft,
                Checks::Pending,
                "#7 draft      WAIT",
                AMBER,
            ),
            (
                CardKind::PullRequest,
                Checks::Passing,
                "#7 PR         PASS",
                GREEN,
            ),
            (CardKind::Unknown, Checks::Unknown, "#7", GREY),
        ] {
            let mut card = card(7);
            card.kind = kind;
            card.checks = checks;
            let canvas = face(&card, false, false);
            assert_eq!(text(&canvas)[0].trim_end(), header);
            assert_eq!(canvas.border, Some(border));
        }
        // A short title leaves its rows empty.
        assert_eq!(text(&face(&card(7), false, false))[2], "");
        assert_eq!(
            [age(59), age(3_599), age(3_600), age(172_799), age(172_800)],
            ["0m", "59m", "1h", "47h", "2d"]
        );
    }

    #[test]
    fn cards_start_in_the_inbox_in_columns_each_covering_the_one_above() {
        let now = Instant::now();
        let mut board = Board::default();
        let cards = cards(14);
        let output = board.update(input(&cards, None, 0.0, now));
        assert_eq!(output.sheets.len(), 15);
        let tray = output.sheets[0].canvas.as_ref().unwrap();
        assert_eq!(text(tray)[0].trim_end(), "INBOX o/r (14)");
        let frame = inbox_frame();
        let local = |index: usize| {
            frame.inverse().transform_point3(
                sheet(&output, index as u64 + 1)
                    .model
                    .unwrap()
                    .w_axis
                    .truncate(),
            )
        };
        // Two columns: twelve, then two, each card lower and nearer than the one above it.
        let (first, second, thirteenth) = (local(0), local(1), local(12));
        assert!(second.y < first.y && second.z > first.z);
        assert!((first.y - second.y - peek()).abs() < 1e-5);
        assert!(thirteenth.x > first.x);
        assert!((thirteenth.y - first.y).abs() < 1e-5);
        // Every card faces home, and stays within the inbox.
        let (width, height) = inbox_size(14);
        for index in 0..14 {
            let point = local(index);
            assert!(
                point.x.abs() < width / 2.0 && point.y.abs() < height / 2.0,
                "{index}: {point}"
            );
        }
        // Only the last card of each column shows in full.
        let covered = |index: usize| {
            text(sheet(&output, index as u64 + 1).canvas.as_ref().unwrap())[4].is_empty()
        };
        assert!(covered(0) && covered(10) && !covered(11) && covered(12) && !covered(13));
        // The status shows on the inbox while there is one.
        let mut stale = cards.clone();
        stale.status = Some("Could not refresh o/r".into());
        let output = board.update(input(&stale, None, 0.0, now));
        assert_eq!(
            text(output.sheets[0].canvas.as_ref().unwrap())[0].trim_end(),
            "INBOX o/r: Could not refresh o/r"
        );
        // An empty inbox still stands.
        let output = board.update(input(&cards_none(), None, 0.0, now));
        assert_eq!(output.sheets.len(), 1);
        // Without cards there is nothing.
        let none = Input {
            cards: None,
            ..input(&cards, None, 0.0, now)
        };
        assert_eq!(board.update(none), Output::default());
    }

    fn cards_none() -> Cards {
        cards(0)
    }

    #[test]
    fn the_ray_points_at_the_nearest_card_it_meets_and_only_while_active() {
        let now = Instant::now();
        let mut board = Board::default();
        let cards = cards(3);
        let models = models(&board.update(input(&cards, None, 0.0, now)), 3);
        // Aimed at the second card's header, the second is met; at its middle, the third, which covers it there.
        let output = board.update(input(&cards, Some(header(models[1])), 0.0, now));
        let (pointed, distance, point) = output.pointed.clone().unwrap();
        assert_eq!(pointed, cards.cards[1].id);
        assert!(point.distance(HOME + header(models[1]).direction * distance) < 1e-4);
        // The pointed card is highlighted.
        let highlighted = sheet(&output, 2).canvas.as_ref().unwrap();
        assert_eq!(highlighted.border, Some(HIGHLIGHT));
        let output = board.update(input(&cards, Some(at(models[1])), 0.0, now));
        assert_eq!(output.pointed.unwrap().0, cards.cards[2].id);
        // Away from the cards, parallel to one, from behind one, or inactive, nothing is.
        let away = Ray::new(HOME, Vec3::Z).unwrap();
        assert!(
            board
                .update(input(&cards, Some(away), 0.0, now))
                .pointed
                .is_none()
        );
        let frame = inbox_frame();
        let along = Ray::new(
            frame.w_axis.truncate() + frame.z_axis.truncate() * 0.004,
            frame.x_axis.truncate(),
        )
        .unwrap();
        assert!(
            board
                .update(input(&cards, Some(along), 0.0, now))
                .pointed
                .is_none()
        );
        let behind = Ray::new(
            frame.w_axis.truncate() - frame.z_axis.truncate(),
            -frame.z_axis.truncate(),
        )
        .unwrap();
        assert!(
            board
                .update(input(&cards, Some(behind), 0.0, now))
                .pointed
                .is_none()
        );
        let inactive = Input {
            active: false,
            ..input(&cards, Some(header(models[1])), 0.0, now)
        };
        assert!(board.update(inactive).pointed.is_none());
    }

    #[test]
    fn the_grip_picks_up_the_card_pointed_at_carries_it_and_puts_it_down() {
        let now = Instant::now();
        let mut board = Board::default();
        let mut cards = cards(3);
        let models = models(&board.update(input(&cards, None, 0.0, now)), 3);
        let id = cards.cards[1].id.clone();
        // Squeezing away from every card picks up nothing.
        let away = Ray::new(HOME, Vec3::Z).unwrap();
        assert!(!board.update(input(&cards, Some(away), 1.0, now)).picked);
        assert_eq!(board.held(), None);
        board.update(input(&cards, Some(away), 0.0, now));
        // A light squeeze does nothing; a firm one picks it up.
        assert!(
            !board
                .update(input(&cards, Some(header(models[1])), 0.6, now))
                .picked
        );
        let picked = board.update(input(&cards, Some(header(models[1])), 0.9, now));
        assert!(picked.picked);
        assert_eq!(picked.pointed, None);
        assert_eq!(board.held(), Some(id.as_str()));
        // It follows the ray at the same distance, facing the viewer, and the inbox closes up behind it.
        let distance = (models[1] * Mat4::from_translation(Vec3::new(0.0, 0.02, 0.0)))
            .w_axis
            .truncate()
            .distance(HOME);
        let right = Ray::new(HOME, Vec3::new(0.6, -0.3, -1.0)).unwrap();
        let carried = board.update(input(&cards, Some(right), 0.7, now));
        assert_eq!(
            text(carried.sheets[0].canvas.as_ref().unwrap())[0].trim_end(),
            "INBOX o/r (2)"
        );
        let model = sheet(&carried, 2).model.unwrap();
        assert!(model.w_axis.truncate().distance(right.at(distance)) < 1e-4);
        assert!(
            model
                .z_axis
                .truncate()
                .dot((HOME - right.at(distance)).normalize())
                > 0.999
        );
        assert_eq!(
            sheet(&carried, 2).canvas.as_ref().unwrap().border,
            Some(HIGHLIGHT)
        );
        assert_eq!(carried.placed, None);
        // Untracked, it stays where it was.
        let untracked = board.update(input(&cards, None, 0.7, now));
        assert_eq!(sheet(&untracked, 2).model, Some(model));
        // Letting go puts it down there, to send, and shows it there until the server says so.
        let put = board.update(input(&cards, Some(right), 0.2, now));
        let (card, placement) = put.placed.clone().unwrap();
        assert_eq!(card, id);
        let placement = placement.unwrap();
        assert!(Vec3::from_array(placement.position).distance(right.at(distance)) < 1e-4);
        assert_eq!(board.held(), None);
        let later = board.update(input(&cards, None, 0.0, now + Duration::from_secs(1)));
        assert!(
            sheet(&later, 2)
                .model
                .unwrap()
                .w_axis
                .truncate()
                .distance(right.at(distance))
                < 1e-4
        );
        // The server agrees: it stays.
        cards.cards[1].placement = Some(placement);
        let agreed = board.update(input(&cards, None, 0.0, now + Duration::from_secs(1)));
        assert!(
            sheet(&agreed, 2)
                .model
                .unwrap()
                .w_axis
                .truncate()
                .distance(right.at(distance))
                < 1e-4
        );
        assert!(board.pending.is_none());
        // Picked up again from where it stands, and let go over the inbox, it goes back in.
        let placed = sheet(&agreed, 2).model.unwrap();
        assert!(
            board
                .update(input(&cards, Some(at(placed)), 0.9, now))
                .picked
        );
        let inbox = Ray::new(HOME, INBOX - HOME).unwrap();
        let back = board.update(input(&cards, Some(inbox), 0.0, now));
        assert_eq!(back.placed, Some((id.clone(), None)));
        let shown = board.update(input(&cards, None, 0.0, now));
        assert_eq!(
            text(shown.sheets[0].canvas.as_ref().unwrap())[0].trim_end(),
            "INBOX o/r (3)"
        );
        // The server never agreeing, it shows as the server has it once the pending time passes.
        let expired = board.update(input(&cards, None, 0.0, now + PENDING));
        assert_eq!(sheet(&expired, 2).model, Some(placed));
        assert!(board.pending.is_none());
    }

    #[test]
    fn a_carried_card_is_put_down_when_its_grip_is_taken_or_dropped_when_it_leaves() {
        let now = Instant::now();
        let mut board = Board::default();
        let cards = cards(2);
        let model = sheet(&board.update(input(&cards, None, 0.0, now)), 1)
            .model
            .unwrap();
        assert!(
            board
                .update(input(&cards, Some(header(model)), 1.0, now))
                .picked
        );
        // The hand menu opens: it is put down where it is.
        let taken = Input {
            active: false,
            ..input(&cards, Some(at(model)), 1.0, now)
        };
        let put = board.update(taken);
        assert_eq!(
            put.placed.as_ref().map(|(card, _)| card.as_str()),
            Some(cards.cards[0].id.as_str())
        );
        // Still held when it opens, the grip picks up nothing more until it is let go and squeezed again.
        assert!(
            !board
                .update(input(&cards, Some(header(model)), 1.0, now))
                .picked
        );
        board.update(input(&cards, Some(header(model)), 0.0, now));
        let second = sheet(&board.update(input(&cards, None, 0.0, now)), 2)
            .model
            .unwrap();
        assert!(
            board
                .update(input(&cards, Some(at(second)), 1.0, now))
                .picked
        );
        // The card closes on GitHub while carried: it is dropped, and nothing is sent.
        let mut fewer = cards.clone();
        fewer.cards.remove(1);
        let gone = board.update(input(&fewer, None, 1.0, now));
        assert_eq!((gone.placed, board.held()), (None, None));
        // A card put down that leaves the board is no longer pending, and leaving the space forgets everything.
        assert!(
            board
                .update(input(&cards, Some(header(model)), 0.0, now))
                .placed
                .is_none()
        );
        let none = Input {
            cards: None,
            ..input(&cards, None, 0.0, now)
        };
        board.update(none);
        assert_eq!((board.held(), board.pending.as_ref()), (None, None));
        // A placement that is not a pose stays in the inbox; a grip that is not a number is let go.
        let mut odd = cards.clone();
        odd.cards[0].placement = Some(Pose {
            position: [0.0, 1.0, -1.0],
            orientation: [0.0; 4],
        });
        let output = board.update(input(&odd, None, f32::NAN, now));
        assert_eq!(
            text(output.sheets[0].canvas.as_ref().unwrap())[0].trim_end(),
            "INBOX o/r (2)"
        );
    }
}
