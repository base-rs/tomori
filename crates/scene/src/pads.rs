// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Teleport pads, as a profile declares them in pairs (see [`crate::locomotion::Pad`]), and teleport targets, objects
//! that take a participant who points at them there (see [`crate::locomotion::Target`]): stepping onto a pad, which
//! counts once the participant has stood on it for [`DWELL`] and disarms the pad they arrive on until they step off
//! it; pointing at either, which the frame step lets the trigger or A choose; the label naming where it goes; and what the
//! renderer draws of each. Pure, so that it is tested without a device; [`crate::locomotion::Travel`] moves the rig.
//! See `design/locomotion.md`.
use glam::{Vec2, Vec3};

use crate::{
    controller_ui::Sheet,
    locomotion::{Locomotion, Pad, REACH, Target},
    pointing::Ray,
    projection::Controls,
};

/// How long a participant stands on a pad before it takes them, in seconds, so that walking across one does not.
pub const DWELL: f32 = 0.4;
/// How far beyond a pad's edge the head goes, horizontally, before the pad a participant arrived on takes them again,
/// in metres, so that swaying at its edge does not send them back.
pub const STEP_OFF: f32 = 0.15;
/// How near a pad's height the room's floor is for the participant to stand on it, in metres.
const LEVEL: f32 = 0.05;
/// How high above a pad the ray points at it, in metres: no higher than the tops of the chevrons drawn above it, so
/// that nothing invisible takes the ray. The renderer's test checks that they agree.
pub const COLUMN: f32 = 0.6;
/// The longest a frame counts towards standing on a pad, in seconds, so that one long frame cannot fire a pad.
const LONGEST_FRAME: f32 = 0.125;
/// How much higher or lower a pad's partner stands for the label and the drawing to say up or down, in metres.
const STOREY: f32 = 0.5;

/// A pad among a profile's pairs, as [`Locomotion::pads`] gives them: the pair's index, and which of its two pads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub pair: usize,
    pub end: usize,
}

impl Key {
    /// The pad it is paired with, which it takes a participant to.
    pub fn partner(self) -> Self {
        Self {
            pair: self.pair,
            end: self.end ^ 1,
        }
    }
}

/// What the ray may choose to travel by: a pad among a profile's pairs, which takes a participant to its partner, or a
/// target, by its index among [`Locomotion::targets`], which takes them to itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Way {
    Pad(Key),
    Target(usize),
}

/// Where `way` takes a participant among `locomotion`'s pads and targets, the room's floor at its height, and which way
/// they face there, if it says; `None` for a way that is not among them.
pub(crate) fn destination(locomotion: &Locomotion, way: Way) -> Option<(Vec3, Option<f32>)> {
    match way {
        Way::Pad(key) => {
            let pairs: Vec<&[Pad; 2]> = locomotion.pads().collect();
            lookup(&pairs, key.partner()).map(|partner| (partner.centre(), partner.yaw()))
        }
        Way::Target(index) => locomotion
            .targets()
            .nth(index)
            .map(|target| (target.arrival(), target.yaw())),
    }
}

/// The pad `key` names among `pairs`, if there is one.
pub(crate) fn lookup<'a>(pairs: &[&'a [Pad; 2]], key: Key) -> Option<&'a Pad> {
    pairs.get(key.pair).and_then(|pair| pair.get(key.end))
}

/// Every pad in `pairs`, keyed.
fn keyed<'a>(pairs: &'a [&'a [Pad; 2]]) -> impl Iterator<Item = (Key, &'a Pad)> {
    pairs.iter().enumerate().flat_map(|(index, pair)| {
        pair.iter()
            .enumerate()
            .map(move |(end, pad)| (Key { pair: index, end }, pad))
    })
}

/// Whether the participant, with their head at `head` and the room's floor at `floor`, stands within `margin` of `pad`.
fn standing(pad: &Pad, head: Vec3, floor: f32, margin: f32) -> bool {
    (floor - pad.centre().y).abs() <= LEVEL && pad.reaches(head, margin)
}

/// Where the participant stands on the pads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Standing {
    /// Not yet seen while pads may be used, as on entering a space or once the hand menu has shown: a pad they are found
    /// on then is disarmed, so that it takes nobody who did not step onto it.
    #[default]
    Unseen,
    /// On no pad.
    Off,
    /// On this pad, for so long.
    On(Key, f32),
    /// On or just beside this pad, which does not take them until they have stepped off it: they arrived on it.
    Disarmed(Key),
}

/// Which pad the participant stands on, between frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Stepping {
    standing: Standing,
}

impl Stepping {
    /// Pads may not be used for now, as while the hand menu shows: the pad the participant is found on next is disarmed.
    pub fn pause(&mut self) {
        self.standing = Standing::Unseen;
    }

    /// The participant arrived on `key`, which is disarmed until they step off it.
    pub fn arrive(&mut self, key: Key) {
        self.standing = Standing::Disarmed(key);
    }

    /// The pad among `pairs` the participant has stood on for [`DWELL`], with their head at `head` and the room's floor
    /// at `floor`, after `dt` seconds more, if one takes them now. A pad stands for its horizontal disc at its height:
    /// they stand on it with the floor within 5 cm of its height and the head above it. A frame counts for no more
    /// than 0.125 s, so that one long frame never takes them. Without a head, nothing changes.
    pub fn update(
        &mut self,
        pairs: &[&[Pad; 2]],
        head: Option<Vec3>,
        floor: f32,
        dt: f32,
    ) -> Option<Key> {
        let head = head.filter(|head| head.is_finite())?;
        let frame = if dt.is_finite() {
            dt.clamp(0.0, LONGEST_FRAME)
        } else {
            0.0
        };
        let on = keyed(pairs)
            .find(|(_, pad)| standing(pad, head, floor, 0.0))
            .map(|(key, _)| key);
        let near =
            |key: Key| lookup(pairs, key).is_some_and(|pad| standing(pad, head, floor, STEP_OFF));
        let mut taken = None;
        self.standing = match self.standing {
            Standing::Unseen => on.map_or(Standing::Off, Standing::Disarmed),
            Standing::Disarmed(key) if near(key) => Standing::Disarmed(key),
            Standing::On(key, held) if on == Some(key) => {
                let held = held + frame;
                if held >= DWELL {
                    taken = Some(key);
                    Standing::Disarmed(key)
                } else {
                    Standing::On(key, held)
                }
            }
            _ => on.map_or(Standing::Off, |key| Standing::On(key, 0.0)),
        };
        taken
    }
}

/// The pad `ray` points at among `locomotion`'s, how far along it and where: the nearest whose disc, or the column
/// [`COLUMN`] high above it, the ray meets, within [`REACH`] of its origin horizontally, unless a declared surface
/// crosses the ray first, from above or below, as the floor of a balcony hides a pad beneath it, and the slab of a deck
/// one above it. A ray from within a pad's column, as a participant standing on it points, meets only its disc.
pub(crate) fn pick(locomotion: &Locomotion, ray: &Ray) -> Option<(Key, f32, Vec3)> {
    let pairs: Vec<&[Pad; 2]> = locomotion.pads().collect();
    let (key, distance) = keyed(&pairs)
        .filter_map(|(key, pad)| meets(pad, ray).map(|distance| (key, distance)))
        .filter(|(_, distance)| {
            let point = ray.at(*distance);
            Vec2::new(point.x - ray.origin.x, point.z - ray.origin.z).length() <= REACH
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))?;
    (!locomotion.surface_crossed(ray, distance - 0.01)).then(|| (key, distance, ray.at(distance)))
}

/// The pad or target `ray` points at among `locomotion`'s, how far along it and where: the nearest, as [`pick`] finds
/// a pad and a target's upright cylinder is met from outside it, within [`REACH`] of the ray's origin horizontally,
/// unless a declared surface crosses the ray first. A target the ray starts over, as one stands on, is not pointed at,
/// since it would take them nowhere, nor one whose reach the ray starts beyond ([`Target::within`]).
pub(crate) fn point(locomotion: &Locomotion, ray: &Ray) -> Option<(Way, f32, Vec3)> {
    let pad = pick(locomotion, ray).map(|(key, distance, at)| (Way::Pad(key), distance, at));
    let target = locomotion
        .targets()
        .enumerate()
        .filter(|(_, target)| !target.over(ray.origin) && target.reached_from(ray.origin))
        .filter_map(|(index, target)| aimed(target, ray).map(|distance| (index, distance)))
        .filter(|(_, distance)| {
            let point = ray.at(*distance);
            Vec2::new(point.x - ray.origin.x, point.z - ray.origin.z).length() <= REACH
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .filter(|(_, distance)| !locomotion.surface_crossed(ray, distance - 0.01))
        .map(|(index, distance)| (Way::Target(index), distance, ray.at(distance)));
    [pad, target]
        .into_iter()
        .flatten()
        .min_by(|(_, a, _), (_, b, _)| a.total_cmp(b))
}

/// How far along `ray` it meets `target`'s upright cylinder, from outside it: its side, or its top from above.
fn aimed(target: &Target, ray: &Ray) -> Option<f32> {
    let (foot, radius) = (target.foot(), target.radius());
    let (bottom, top) = (foot.y, foot.y + target.height());
    let offset = Vec2::new(ray.origin.x - foot.x, ray.origin.z - foot.z);
    let across = Vec2::new(ray.direction.x, ray.direction.z);
    let a = across.length_squared();
    let b = offset.dot(across);
    let c = offset.length_squared() - radius * radius;
    let side = (a > f32::EPSILON && c > 0.0 && b * b - a * c >= 0.0)
        .then(|| (-b - (b * b - a * c).sqrt()) / a)
        .filter(|distance| *distance >= 0.0 && (bottom..=top).contains(&ray.at(*distance).y));
    let onto = (ray.origin.y > top && ray.direction.y < 0.0)
        .then(|| (top - ray.origin.y) / ray.direction.y)
        .filter(|distance| target.over(ray.at(*distance)));
    side.or(onto)
}

/// How far along `ray` it meets `pad`'s column, from outside it, or its disc, from within the column above it.
fn meets(pad: &Pad, ray: &Ray) -> Option<f32> {
    let (centre, radius) = (pad.centre(), pad.radius());
    let top = centre.y + COLUMN;
    let within = |point: Vec3| pad.reaches(point, 0.0);
    // Where the ray crosses the height `height` going down, if it does there within the pad.
    let down_to = |height: f32| {
        let distance = (height - ray.origin.y) / ray.direction.y;
        (ray.direction.y < 0.0 && distance >= 0.0 && within(ray.at(distance))).then_some(distance)
    };
    let inside = within(ray.origin) && (centre.y..=top).contains(&ray.origin.y);
    if inside {
        return down_to(centre.y);
    }
    // Into its side, between its disc and its top.
    let offset = Vec2::new(ray.origin.x - centre.x, ray.origin.z - centre.z);
    let across = Vec2::new(ray.direction.x, ray.direction.z);
    let a = across.length_squared();
    let b = offset.dot(across);
    let c = offset.length_squared() - radius * radius;
    let side = (a > f32::EPSILON && b * b - a * c >= 0.0)
        .then(|| (-b - (b * b - a * c).sqrt()) / a)
        .filter(|distance| *distance >= 0.0 && (centre.y..=top).contains(&ray.at(*distance).y));
    side.or_else(|| down_to(top).filter(|_| ray.origin.y > top))
}

/// The label beside the pad `key` among `pairs`, pointed at `at`, facing `viewer`: where it goes, by the name of the pad
/// paired with it, and whether that is up or down, and how to go with `controls`.
pub(crate) fn label(
    pairs: &[&[Pad; 2]],
    key: Key,
    at: Vec3,
    viewer: Vec3,
    controls: Controls,
) -> Option<Sheet> {
    use crate::projection::LABEL_COLUMNS;
    let pair = pairs.get(key.pair)?;
    // A pad of the pair, `end` 0 or 1, has the other as its partner.
    let (from, to) = (pair.get(key.end)?, &pair[key.end ^ 1]);
    let name = to.name().unwrap_or("Teleport");
    // The name is cut first, so that the arrow, which says which way, is never what is lost.
    let destination = match direction(from, to) {
        Some(up) => format!(
            "{} {}",
            crate::ui_text::clip(name, LABEL_COLUMNS - 2),
            if up { '↑' } else { '↓' }
        ),
        None => crate::ui_text::clip(name, LABEL_COLUMNS),
    };
    Some(sheet(destination, at, viewer, controls))
}

/// The label beside whatever `way` names among `locomotion`'s, pointed at `at`, facing `viewer`: a pad's, as [`label`]
/// gives it, or a target's, which names it, or Teleport, and how to go with `controls`.
pub(crate) fn label_for(
    locomotion: &Locomotion,
    way: Way,
    at: Vec3,
    viewer: Vec3,
    controls: Controls,
) -> Option<Sheet> {
    match way {
        Way::Pad(key) => {
            let pairs: Vec<&[Pad; 2]> = locomotion.pads().collect();
            label(&pairs, key, at, viewer, controls)
        }
        Way::Target(index) => {
            let target = locomotion.targets().nth(index)?;
            Some(sheet(
                crate::ui_text::clip(
                    target.name().unwrap_or("Teleport"),
                    crate::projection::LABEL_COLUMNS,
                ),
                at,
                viewer,
                controls,
            ))
        }
    }
}

/// A label saying `destination` and how to go with `controls`, beside `at`, facing `viewer`.
fn sheet(destination: String, at: Vec3, viewer: Vec3, controls: Controls) -> Sheet {
    use crate::projection::{HIGHLIGHT, LABEL_COLUMNS, LABEL_PIXEL};
    use crate::transcript::{BRIGHT, TEXT};
    use crate::ui_text::{Row, Span, Style, cut};
    let control = match controls {
        Controls::Controllers => "Trigger",
        Controls::Keyboard => "Click",
    };
    let mut canvas = crate::canvas::Canvas::new(LABEL_COLUMNS, LABEL_PIXEL);
    canvas.background = [0.0, 0.0, 0.0, 0.88];
    canvas.border = Some(HIGHLIGHT);
    canvas.rows = vec![
        Row::text(&destination, Style::bold(BRIGHT), LABEL_COLUMNS),
        Row::new(cut(
            &[
                Span::new(format!("{control}  "), Style::bold(BRIGHT)),
                Span::new("go", Style::plain(TEXT)),
            ],
            LABEL_COLUMNS,
        )),
    ];
    let towards = (viewer - at).try_normalize().unwrap_or(Vec3::Z);
    let (_, height) = canvas.size();
    Sheet {
        model: Some(crate::recording::facing(
            at + towards * 0.08 + Vec3::Y * (height / 2.0 + 0.03),
            viewer,
            1.0,
        )),
        canvas: Some(crate::frame_timing::drawn(canvas)),
    }
}

/// Where a ring is drawn while `way` is pointed at: where a target takes the participant, as the ring where a teleport
/// would land is; nothing for a pad, which is drawn itself.
pub(crate) fn ring(locomotion: &Locomotion, way: Option<Way>) -> Option<Vec3> {
    match way? {
        Way::Pad(_) => None,
        target @ Way::Target(_) => destination(locomotion, target).map(|(arrival, _)| arrival),
    }
}

/// Whether `to` stands a storey above `from`, `Some(true)`, below it, `Some(false)`, or about level with it.
fn direction(from: &Pad, to: &Pad) -> Option<bool> {
    let rise = to.centre().y - from.centre().y;
    (rise.abs() >= STOREY).then_some(rise > 0.0)
}

/// A pad as the renderer draws it, in the space's scene coordinates.
///
/// Non-exhaustive, so that what is drawn of a pad can grow; make one with [`Shown::new`], and set its fields.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Shown {
    pub centre: Vec3,
    pub radius: f32,
    /// Whether the pad it takes a participant to stands a storey above it, `Some(true)`, below it, `Some(false)`, or
    /// about level, which the arrow above it says.
    pub up: Option<bool>,
    /// The ray points at it.
    pub highlighted: bool,
}

impl Shown {
    /// A pad at `centre`, `radius` across, whose partner is higher, `Some(true)`, lower, `Some(false)`, or about level,
    /// and which the ray points at, or not.
    pub fn new(centre: Vec3, radius: f32, up: Option<bool>, highlighted: bool) -> Self {
        Self {
            centre,
            radius,
            up,
            highlighted,
        }
    }
}

/// Every pad of `locomotion`'s that a participant may use, as drawn, with the one `pointed` names highlighted. Targets
/// are the environment's to draw.
pub(crate) fn shown(locomotion: &Locomotion, pointed: Option<Way>) -> Vec<Shown> {
    let highlighted = match pointed {
        Some(Way::Pad(key)) => Some(key),
        _ => None,
    };
    let pairs: Vec<&[Pad; 2]> = locomotion.pads().collect();
    keyed(&pairs)
        .map(|(key, pad)| Shown {
            centre: pad.centre(),
            radius: pad.radius(),
            up: lookup(&pairs, key.partner()).and_then(|partner| direction(pad, partner)),
            highlighted: highlighted == Some(key),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locomotion::Surface;
    use glam::Vec2;

    const FLOOR: Key = Key { pair: 0, end: 0 };
    const RACKS: Key = Key { pair: 0, end: 1 };
    const DECK: Key = Key { pair: 1, end: 1 };

    /// A main floor with a pad at its origin, racks 4.5 m below with theirs, and a deck 5.4 m above, its pad paired
    /// with a second on the floor, level with nothing but itself; and a balcony over part of the floor.
    fn storeys() -> Locomotion {
        Locomotion::fly(
            vec![
                Surface::rectangle(Vec3::ZERO, Vec2::new(30.0, 30.0)),
                Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::new(30.0, 30.0)),
                Surface::rectangle(Vec3::new(0.0, 5.4, -8.0), Vec2::new(6.0, 4.0)),
                Surface::rectangle(Vec3::new(8.0, 3.0, 0.0), Vec2::new(2.0, 2.0)),
            ],
            8.0,
        )
        .with_pads([
            [
                Pad::at(Vec3::ZERO),
                Pad::at(Vec3::new(4.0, -4.5, 0.0)).named("Racks"),
            ],
            [
                Pad::at(Vec3::new(-4.0, 0.0, 0.0)).named("Main floor"),
                Pad::at(Vec3::new(0.0, 5.4, -8.0)).named("Deck"),
            ],
        ])
    }

    fn ray(origin: Vec3, towards: Vec3) -> Ray {
        Ray::new(origin, towards - origin).unwrap()
    }

    #[test]
    fn a_pad_s_partner_is_the_other_of_its_pair() {
        assert_eq!(FLOOR.partner(), RACKS);
        assert_eq!(RACKS.partner(), FLOOR);
        let storeys = storeys();
        let pairs: Vec<_> = storeys.pads().collect();
        assert_eq!(lookup(&pairs, DECK).unwrap().name(), Some("Deck"));
        assert_eq!(lookup(&pairs, Key { pair: 2, end: 0 }), None);
        assert_eq!(lookup(&pairs, Key { pair: 0, end: 2 }), None);
    }

    #[test]
    fn a_pad_takes_a_participant_who_stands_on_it_for_the_dwell_once() {
        let storeys = storeys();
        let pairs: Vec<_> = storeys.pads().collect();
        let on = Vec3::new(0.2, 1.7, 0.1);
        let off = Vec3::new(1.0, 1.7, 0.0);
        let mut stepping = Stepping::default();
        let step = |stepping: &mut Stepping, head: Option<Vec3>, floor: f32| {
            stepping.update(&pairs, head, floor, 0.1)
        };
        // Found on it first, it is disarmed until stepped off, by more than 15 cm beyond its edge.
        for head in [on, Vec3::new(0.6, 1.7, 0.0), on, on, on, on, on] {
            assert_eq!(step(&mut stepping, Some(head), 0.0), None);
        }
        step(&mut stepping, Some(off), 0.0);
        // Back on it, it takes them after 0.4 s, and only once.
        let taken: Vec<_> = (0..8).map(|_| step(&mut stepping, Some(on), 0.0)).collect();
        assert_eq!(taken[..4], [None; 4]);
        assert_eq!(taken[4], Some(FLOOR));
        assert_eq!(taken[5..], [None; 3]);
        // Stepping off before the dwell starts it again, and without a head nothing changes.
        step(&mut stepping, Some(off), 0.0);
        for _ in 0..3 {
            step(&mut stepping, Some(on), 0.0);
        }
        step(&mut stepping, None, 0.0);
        step(&mut stepping, Some(Vec3::NAN), 0.0);
        step(&mut stepping, Some(off), 0.0);
        for _ in 0..4 {
            assert_eq!(step(&mut stepping, Some(on), 0.0), None);
        }
        assert_eq!(step(&mut stepping, Some(on), 0.0), Some(FLOOR));
        // Hovering above it, or below it, a participant does not stand on it.
        step(&mut stepping, Some(off), 0.0);
        for _ in 0..10 {
            assert_eq!(step(&mut stepping, Some(on), 0.1), None);
            assert_eq!(step(&mut stepping, Some(on), -0.06), None);
        }
        // Paused, as while the hand menu shows, the pad they stand on is disarmed.
        step(&mut stepping, Some(off), 0.0);
        for _ in 0..3 {
            step(&mut stepping, Some(on), 0.0);
        }
        stepping.pause();
        for _ in 0..10 {
            assert_eq!(step(&mut stepping, Some(on), 0.0), None);
        }
        // Arrived on the racks' pad, it is disarmed; another pad then takes them as ever.
        stepping.arrive(RACKS);
        let racks = Vec3::new(4.0, -2.8, 0.0);
        for _ in 0..10 {
            assert_eq!(step(&mut stepping, Some(racks), -4.5), None);
        }
        let other = Vec3::new(-4.0, 1.7, 0.0);
        let taken: Vec<_> = (0..5)
            .map(|_| step(&mut stepping, Some(other), 0.0))
            .collect();
        assert_eq!(taken[4], Some(Key { pair: 1, end: 0 }));
        // A pad that has gone, as the profile changes, disarms nothing.
        stepping.arrive(Key { pair: 5, end: 0 });
        step(&mut stepping, Some(on), 0.0);
        let taken: Vec<_> = (0..4).map(|_| step(&mut stepping, Some(on), 0.0)).collect();
        assert_eq!(taken[3], Some(FLOOR));
    }

    #[test]
    fn the_ray_points_at_the_nearest_pad_s_column_or_its_disc_from_within() {
        let storeys = storeys();
        let eye = Vec3::new(0.0, 1.5, 3.0);
        // Into the side of the floor's pad's column, from in front.
        let (key, distance, point) = pick(&storeys, &ray(eye, Vec3::new(0.0, 0.4, 0.0))).unwrap();
        assert_eq!(key, FLOOR);
        assert!(
            (point.z - crate::locomotion::PAD_RADIUS).abs() < 1e-4,
            "{point}"
        );
        assert!((distance - eye.distance(point)).abs() < 1e-4);
        // Above its column, its top; from below its disc, pointing up past it, or level beside it, nothing.
        let (key, _, point) = pick(
            &storeys,
            &ray(Vec3::new(0.0, 4.0, 1.0), Vec3::new(0.0, 0.0, 0.0)),
        )
        .unwrap();
        assert_eq!(key, FLOOR);
        assert!((point.y - COLUMN).abs() < 1e-4, "{point}");
        assert_eq!(
            pick(
                &storeys,
                &ray(Vec3::new(0.0, -3.0, 1.0), Vec3::new(0.0, 0.5, 0.0))
            ),
            None
        );
        assert_eq!(
            pick(
                &storeys,
                &ray(Vec3::new(0.0, 1.5, 1.0), Vec3::new(0.0, 4.0, -1.0))
            ),
            None
        );
        assert_eq!(
            pick(
                &storeys,
                &ray(Vec3::new(2.0, 1.5, 0.0), Vec3::new(3.0, 1.5, 0.0))
            ),
            None
        );
        // Straight down onto its top, and straight down beside it, which meets nothing.
        let down = Ray::new(Vec3::new(0.1, 3.0, 0.0), Vec3::NEG_Y).unwrap();
        assert_eq!(pick(&storeys, &down).unwrap().0, FLOOR);
        let beside = Ray::new(Vec3::new(1.0, 3.0, 0.0), Vec3::NEG_Y).unwrap();
        assert_eq!(pick(&storeys, &beside), None);
        // Away from it, behind the ray, nothing.
        assert_eq!(pick(&storeys, &ray(eye, Vec3::new(0.0, 0.8, 6.0))), None);
        // From within its column, as a hand held low over it, the ray meets only its disc, pointing down at it, and
        // nothing pointing ahead.
        let standing = Vec3::new(0.1, 0.6, 0.0);
        let (key, _, point) = pick(&storeys, &ray(standing, Vec3::new(0.0, 0.0, -0.2))).unwrap();
        assert_eq!(key, FLOOR);
        assert!(point.y.abs() < 1e-4);
        assert_eq!(
            pick(&storeys, &ray(standing, Vec3::new(0.0, 0.6, -3.0))),
            None
        );
        // Of two, the nearer.
        let along = ray(Vec3::new(-8.0, 0.6, 0.0), Vec3::new(1.0, 0.6, 0.0));
        assert_eq!(pick(&storeys, &along).unwrap().0, Key { pair: 1, end: 0 });
        // Beyond 15 m horizontally, nothing.
        let far = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 40.0)]).with_pads([[
            Pad::at(Vec3::new(0.0, 0.0, -18.0)),
            Pad::at(Vec3::new(0.0, 0.0, 2.0)),
        ]]);
        assert_eq!(
            pick(
                &far,
                &ray(Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.0, 0.5, -18.0))
            ),
            None
        );
        // A pad beneath a balcony is hidden from above it, and seen from beneath it.
        let deck_eye = Vec3::new(0.0, 7.0, -6.5);
        let hidden = Locomotion::fly(
            vec![
                Surface::rectangle(Vec3::ZERO, Vec2::new(30.0, 30.0)),
                Surface::rectangle(Vec3::new(0.0, 5.4, -8.0), Vec2::new(6.0, 4.0)),
            ],
            8.0,
        )
        .with_pads([[
            Pad::at(Vec3::new(0.0, 0.0, -8.0)),
            Pad::at(Vec3::new(0.0, 0.0, 4.0)),
        ]]);
        assert_eq!(
            pick(&hidden, &ray(deck_eye, Vec3::new(0.0, 0.0, -8.0))),
            None
        );
        assert_eq!(
            pick(
                &hidden,
                &ray(Vec3::new(0.0, 1.5, -2.0), Vec3::new(0.0, 0.0, -8.0))
            )
            .unwrap()
            .0,
            FLOOR
        );
        // Nothing for a profile of none.
        assert_eq!(pick(&Locomotion::None, &ray(eye, Vec3::ZERO)), None);
    }

    #[test]
    fn the_label_names_where_a_pad_goes_and_how_to_go() {
        let storeys = storeys();
        let pairs: Vec<_> = storeys.pads().collect();
        let viewer = Vec3::new(0.0, 1.6, 3.0);
        let text = |key: Key, controls: Controls| {
            let sheet = label(&pairs, key, Vec3::new(0.0, 0.6, 0.5), viewer, controls).unwrap();
            assert!(sheet.model.unwrap().is_finite());
            sheet.canvas.unwrap().plain()
        };
        assert!(text(FLOOR, Controls::Controllers).starts_with("Racks ↓"));
        assert!(text(FLOOR, Controls::Controllers).contains("Trigger  go"));
        assert!(text(RACKS, Controls::Keyboard).starts_with("Teleport ↑"));
        assert!(text(RACKS, Controls::Keyboard).contains("Click  go"));
        assert!(text(Key { pair: 1, end: 0 }, Controls::Controllers).starts_with("Deck ↑"));
        // Level with each other, no arrow; a pad not among them, no label.
        let level = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 10.0)])
            .with_pads([[Pad::at(Vec3::ZERO), Pad::at(Vec3::X * 3.0).named("Door")]]);
        let level: Vec<_> = level.pads().collect();
        let sheet = label(&level, FLOOR, Vec3::ZERO, viewer, Controls::Controllers).unwrap();
        assert!(sheet.canvas.unwrap().plain().starts_with("Door\n"));
        for missing in [DECK, Key { pair: 0, end: 2 }] {
            assert!(label(&level, missing, Vec3::ZERO, viewer, Controls::Controllers).is_none());
        }
        // Every arrow is drawn as itself.
        for arrow in ['↑', '↓'] {
            assert!(tomori_text::fonts::covers(arrow), "{arrow}");
        }
    }

    #[test]
    fn every_pad_is_shown_with_which_way_it_goes_and_the_one_pointed_at_highlighted() {
        let shown = shown(&storeys(), Some(Way::Pad(RACKS)));
        assert_eq!(shown.len(), 4);
        assert_eq!(
            shown[0],
            Shown {
                centre: Vec3::ZERO,
                radius: crate::locomotion::PAD_RADIUS,
                up: Some(false),
                highlighted: false,
            }
        );
        assert_eq!((shown[1].up, shown[1].highlighted), (Some(true), true));
        assert_eq!(shown[3].up, Some(false));
        let level = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 10.0)])
            .with_pads([[Pad::at(Vec3::ZERO), Pad::at(Vec3::new(3.0, 0.0, 0.0))]]);
        assert!(
            super::shown(&level, None)
                .iter()
                .all(|pad| pad.up.is_none() && !pad.highlighted)
        );
        assert!(super::shown(&Locomotion::None, None).is_empty());
    }

    #[test]
    fn one_long_frame_never_takes_a_participant_who_has_only_just_stepped_on() {
        let storeys = storeys();
        let pairs: Vec<_> = storeys.pads().collect();
        let on = Vec3::new(0.2, 1.7, 0.1);
        // A hitch of half a second, or more, or nonsense, counts for no more than the longest frame, or nothing: of the
        // frames after it, in a normal length, it still takes the dwell's remainder.
        for hitch in [0.5, 60.0, f32::INFINITY, f32::NAN, -1.0] {
            let mut stepping = Stepping::default();
            stepping.update(&pairs, Some(Vec3::new(3.0, 1.7, 3.0)), 0.0, 0.1);
            assert_eq!(stepping.update(&pairs, Some(on), 0.0, 0.1), None);
            assert_eq!(
                stepping.update(&pairs, Some(on), 0.0, hitch),
                None,
                "{hitch}"
            );
            let waited = (0..8)
                .take_while(|_| stepping.update(&pairs, Some(on), 0.0, 0.125).is_none())
                .count();
            assert!((2..=3).contains(&waited), "{hitch} {waited}");
        }
    }

    #[test]
    fn a_long_name_is_cut_before_its_arrow_and_a_pad_on_a_deck_is_hidden_by_its_slab_from_below() {
        let long = Locomotion::teleport(vec![
            Surface::rectangle(Vec3::ZERO, Vec2::new(10.0, 10.0)),
            Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::new(10.0, 10.0)),
        ])
        .with_pads([[
            Pad::at(Vec3::ZERO),
            Pad::at(Vec3::new(2.0, -4.5, 0.0)).named("あ".repeat(24)),
        ]]);
        let pairs: Vec<_> = long.pads().collect();
        let viewer = Vec3::new(0.0, 1.6, 3.0);
        let sheet = label(&pairs, FLOOR, Vec3::ZERO, viewer, Controls::Controllers).unwrap();
        let first = sheet
            .canvas
            .unwrap()
            .plain()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        assert!(first.ends_with("… ↓"), "{first}");
        assert!(crate::ui_text::width(&first) <= crate::projection::LABEL_COLUMNS);
        // Level, the whole width is the name's.
        let level = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 10.0)]).with_pads([[
            Pad::at(Vec3::ZERO),
            Pad::at(Vec3::X * 3.0).named("あ".repeat(24)),
        ]]);
        let pairs: Vec<_> = level.pads().collect();
        let sheet = label(&pairs, FLOOR, Vec3::ZERO, viewer, Controls::Controllers).unwrap();
        let first = sheet
            .canvas
            .unwrap()
            .plain()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        assert!(crate::ui_text::width(&first) <= crate::projection::LABEL_COLUMNS);
        // The deck's pad, 5.4 m up, is not pointed at from beneath its slab, from the floor, but is from above.
        let storeys = storeys();
        let deck = Vec3::new(0.0, 5.4, -8.0);
        let below = ray(Vec3::new(0.0, 1.5, -3.0), deck + Vec3::Y * 0.3);
        assert_eq!(pick(&storeys, &below), None);
        let above = ray(Vec3::new(0.0, 7.0, -3.0), deck + Vec3::Y * 0.3);
        assert_eq!(pick(&storeys, &above).unwrap().0, DECK);
    }

    /// The floor of [`storeys`], its pads, and a rock standing in a pool sunk into it, 6 m ahead, its top 0.4 m above the
    /// floor, and a stone just behind the floor's pad that takes a participant back onto the floor beside it.
    fn pool() -> Locomotion {
        storeys().with_targets([
            Target::object(Vec3::new(0.0, -0.6, -6.0), 0.8, 1.0).named("Rock"),
            Target::object(Vec3::new(0.0, 0.0, 1.2), 0.25, 1.0).arriving(Vec3::new(1.0, 0.0, 1.2)),
        ])
    }

    #[test]
    fn the_ray_points_at_a_target_s_side_or_top_from_outside_it_and_the_nearer_of_a_target_and_a_pad()
     {
        let pool = pool();
        let eye = Vec3::new(0.0, 1.6, 0.5);
        // Its side, between its foot and its top, as the ray meets it first.
        let (way, distance, at) = point(&pool, &ray(eye, Vec3::new(0.0, 0.0, -5.2))).unwrap();
        assert_eq!(way, Way::Target(0));
        assert!((at.z + 5.2).abs() < 1e-4 && (distance - eye.distance(at)).abs() < 1e-4);
        // Its top, from above, over it; past its top, beside it, or from below its foot, nothing.
        let (way, _, at) = point(&pool, &ray(eye, Vec3::new(0.3, 0.4, -6.2))).unwrap();
        assert_eq!(way, Way::Target(0));
        assert!((at.y - 0.4).abs() < 1e-4, "{at}");
        let up = Ray::new(Vec3::new(0.0, -3.0, -1.0), Vec3::new(0.0, 0.5, -1.0)).unwrap();
        for nothing in [
            ray(eye, Vec3::new(0.0, 0.5, -8.0)),
            ray(eye, Vec3::new(2.0, 0.0, -6.0)),
            Ray::new(Vec3::new(0.0, 2.0, -3.0), Vec3::Y).unwrap(),
            up,
        ] {
            assert_eq!(point(&pool, &nothing), None, "{nothing:?}");
        }
        // From over it, as one standing on it points, nothing of it, at its top or its side.
        let standing = Vec3::new(0.2, 1.9, -6.0);
        assert_eq!(
            point(&pool, &ray(standing, Vec3::new(0.0, 0.4, -6.0))),
            None
        );
        // The nearer of a pad and a target, either way round.
        let towards = ray(Vec3::new(0.0, 0.5, -1.5), Vec3::new(0.0, 0.4, 0.0));
        assert_eq!(point(&pool, &towards).unwrap().0, Way::Pad(FLOOR));
        let from = ray(Vec3::new(0.0, 0.8, 3.0), Vec3::new(0.0, 0.3, 0.0));
        assert_eq!(point(&pool, &from).unwrap().0, Way::Target(1));
        // Beyond 15 m horizontally, or behind a surface it crosses first, nothing.
        let far = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 40.0)])
            .with_targets([Target::object(Vec3::new(0.0, 0.0, -18.0), 1.0, 2.0)]);
        assert_eq!(
            point(
                &far,
                &ray(Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.0, 1.0, -17.0))
            ),
            None
        );
        let under = Locomotion::teleport(vec![
            Surface::disc(Vec3::ZERO, 40.0),
            Surface::rectangle(Vec3::new(0.0, 1.0, -4.0), Vec2::new(4.0, 4.0)),
        ])
        .with_targets([Target::object(Vec3::new(0.0, 0.0, -4.0), 0.5, 0.5)]);
        assert_eq!(
            point(
                &under,
                &ray(Vec3::new(0.0, 3.0, 0.0), Vec3::new(0.0, 0.4, -4.0))
            ),
            None
        );
        assert_eq!(point(&Locomotion::None, &ray(eye, Vec3::ZERO)), None);
    }

    #[test]
    fn a_target_with_a_reach_is_pointed_at_only_from_within_it() {
        // A fall reached only from the rock before it, within 4 m of its foot, and a stone pointed at from anywhere.
        let fall = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 40.0)]).with_targets([
            Target::object(Vec3::new(0.0, 0.0, -8.0), 1.0, 4.0)
                .within(4.0)
                .arriving(Vec3::new(0.0, 0.0, 2.0)),
        ]);
        let target = fall.targets().next().unwrap();
        assert_eq!(target.reach(), Some(4.0));
        let near = Vec3::new(0.5, 1.6, -5.0);
        assert!(target.reached_from(near) && !target.reached_from(Vec3::new(0.0, 1.6, 0.0)));
        let (way, ..) = point(&fall, &ray(near, Vec3::new(0.0, 2.0, -7.0))).unwrap();
        assert_eq!(way, Way::Target(0));
        // From the shore 8 m away, the same fall is pointed at no more, nor from high above it.
        let shore = Vec3::new(0.0, 1.6, 0.0);
        assert_eq!(point(&fall, &ray(shore, Vec3::new(0.0, 2.0, -7.0))), None);
        let above = Vec3::new(0.0, 8.0, -5.0);
        assert_eq!(point(&fall, &ray(above, Vec3::new(0.0, 2.0, -7.0))), None);
        let anywhere = Target::object(Vec3::ZERO, 0.5, 1.0);
        assert!(anywhere.reach().is_none() && anywhere.reached_from(Vec3::splat(9.0)));
    }

    #[test]
    fn a_target_s_label_names_it_and_its_ring_stands_where_it_takes_the_participant() {
        let pool = pool();
        let viewer = Vec3::new(0.0, 1.6, 3.0);
        let text = |way: Way, controls: Controls| {
            label_for(&pool, way, Vec3::new(0.0, 0.4, -5.2), viewer, controls)
                .map(|sheet| sheet.canvas.unwrap().plain())
        };
        assert!(
            text(Way::Target(0), Controls::Controllers)
                .unwrap()
                .starts_with("Rock\n")
        );
        assert!(
            text(Way::Target(0), Controls::Keyboard)
                .unwrap()
                .contains("Click  go")
        );
        assert!(
            text(Way::Target(1), Controls::Controllers)
                .unwrap()
                .starts_with("Teleport\n")
        );
        assert!(
            text(Way::Pad(FLOOR), Controls::Controllers)
                .unwrap()
                .starts_with("Racks ↓")
        );
        assert_eq!(text(Way::Target(2), Controls::Controllers), None);
        // Its ring, where it takes them; a pad's is itself, and nothing pointed at, nothing.
        let top = Vec3::new(0.0, 0.4, -6.0);
        assert!(ring(&pool, Some(Way::Target(0))).unwrap().distance(top) < 1e-5);
        assert_eq!(
            ring(&pool, Some(Way::Target(1))),
            Some(Vec3::new(1.0, 0.0, 1.2))
        );
        assert_eq!(ring(&pool, Some(Way::Pad(FLOOR))), None);
        assert_eq!(ring(&pool, None), None);
        assert_eq!(ring(&pool, Some(Way::Target(5))), None);
        // Where each goes, and the pads drawn, the target pointed at highlighting none.
        assert_eq!(
            destination(&pool, Way::Pad(FLOOR)).unwrap().0,
            Vec3::new(4.0, -4.5, 0.0)
        );
        let (arrival, yaw) = destination(&pool, Way::Target(0)).unwrap();
        assert!(arrival.distance(top) < 1e-5 && yaw.is_none());
        assert!(
            shown(&pool, Some(Way::Target(0)))
                .iter()
                .all(|pad| !pad.highlighted)
        );
    }

    #[test]
    fn a_shown_pad_is_made_with_its_fields_and_they_may_be_changed() {
        let mut pad = Shown::new(Vec3::Y, 0.5, Some(true), false);
        assert_eq!(
            (pad.centre, pad.radius, pad.up, pad.highlighted),
            (Vec3::Y, 0.5, Some(true), false)
        );
        pad.highlighted = true;
        pad.radius = 1.0;
        assert_eq!(pad, Shown::new(Vec3::Y, 1.0, Some(true), true));
    }
}
