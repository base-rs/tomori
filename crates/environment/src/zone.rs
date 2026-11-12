// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where participants are, as a server side learns it: the proximity zones it declares, each of whose occupancy the
//! server tells it as an [`Intent`](crate::Intent) only the server makes. See `design/environments.md`.
use std::fmt;
use std::time::Duration;

/// How engaged a participant is, as their client last reported: worn and with input, worn but distracted, or away.
pub use tomori_protocol::Engagement;

/// The most zones a side declares: a declaration of more is refused.
pub const ZONE_LIMIT: usize = 16;

/// The longest name of a zone, in bytes: as long as an object's reference, and of the same characters
/// ([`crate::valid_reference`]).
pub const ZONE_NAME_BYTES: usize = crate::REFERENCE_BYTES;

/// The least and the most radius of a zone, in metres: from about an arm's length, as an anchor's
/// `notify_within` begins, to as far as anchors stand from home.
pub const LEAST_ZONE_RADIUS: f32 = 0.3;
pub const MOST_ZONE_RADIUS: f32 = 20.0;

/// How far beyond its radius someone goes, by default, in metres, before they are no longer within a zone, so that
/// standing at its edge does not take them in and out: the margin an anchor's `notify_within` keeps, so that the agent's
/// proximity and an environment's agree.
pub const ZONE_MARGIN: f32 = 0.5;

/// The widest margin a zone may keep beyond its radius, in metres.
pub const MOST_ZONE_MARGIN: f32 = 5.0;

/// The longest a zone may wait, its occupancy changed, before it is told: a minute.
pub const MOST_ZONE_DWELL: Duration = Duration::from_secs(60);

/// How often the server takes a side's declaration of zones at most: one that comes sooner waits, the latest replacing
/// the one before, and is taken at the hub's next sweep once it may be, as an anchor's changes are.
pub const ZONE_INTERVAL: Duration = Duration::from_millis(900);

/// The most a cylinder's band reaches below or above its point, in metres.
pub const MOST_ZONE_HEIGHT: f32 = 20.0;

/// The most participants a zone counts.
pub const PRESENCE_LIMIT: usize = 64;

/// How long a participant's report stays current: their client reports with every frame, so one not heard from for this
/// long has stopped, as while a headset sleeps or is lifted off. A zone counts nobody whose report is this old, and the
/// agent's `presence` marks them stale; a side may judge [`crate::Presence::at`] by it too. The server's own constant
/// is this one.
pub const PRESENCE_STALE: Duration = Duration::from_secs(5);

/// What a zone is around: a point in the environment's own coordinates, or one of the anchors or objects the
/// environment declares, places or describes, by its name or identity, wherever it stands as the zone is looked at. A
/// zone around an anchor or object that is not there, or an object without a position, holds nobody.
#[derive(Clone, Debug, PartialEq)]
pub enum Around {
    /// A point in the environment's own coordinates.
    Point([f32; 3]),
    /// The point of a cluster anchor the environment declares or its side placed, by its name.
    Anchor(String),
    /// Where an object the side describes stands, by its identity, as [`crate::Object::at`] gives it.
    Object(String),
}

impl Around {
    /// Around `point`, in the environment's own coordinates.
    pub fn point(point: [f32; 3]) -> Self {
        Self::Point(point)
    }

    /// Around the anchor `name`, wherever it stands.
    pub fn anchor(name: impl Into<String>) -> Self {
        Self::Anchor(name.into())
    }

    /// Around the object of identity `id`, wherever the side describes it standing.
    pub fn object(id: impl Into<String>) -> Self {
        Self::Object(id.into())
    }
}

/// The shape of a zone about its point.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Shape {
    /// Everything within `radius` of the point.
    Sphere { radius: f32 },
    /// A vertical cylinder: everything within `radius` of the point across the floor, from `below` beneath it to
    /// `above` over it.
    Cylinder { radius: f32, below: f32, above: f32 },
}

/// A named region of the environment whose occupancy the side is told: who stands within it is counted by where their
/// head is, as their client last reported it, entering at its radius and leaving only beyond it and its margin
/// ([`ZONE_MARGIN`] unless it says), and a change from nobody to somebody, or back, told only once it has lasted its
/// dwell, none unless it says. See [`crate::Space::declare_zones`].
///
/// ```
/// use std::time::Duration;
/// use tomori_environment::{Around, Zone};
///
/// let console = Zone::cylinder("console/2", Around::anchor("console/2"), 1.5, 0.5, 2.5)
///     .leaving_beyond(0.75)
///     .dwelling(Duration::from_secs(2));
/// assert_eq!(console.name(), "console/2");
/// assert_eq!(console.margin(), 0.75);
/// assert_eq!(console.dwell(), Duration::from_secs(2));
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Zone {
    name: String,
    around: Around,
    shape: Shape,
    margin: f32,
    dwell: Duration,
}

impl Zone {
    /// The zone `name`: everything within `radius` of what it is `around`.
    pub fn sphere(name: impl Into<String>, around: Around, radius: f32) -> Self {
        Self::new(name, around, Shape::Sphere { radius })
    }

    /// The zone `name`: everything within `radius` of what it is `around` across the floor, from `below` beneath its
    /// point to `above` over it.
    pub fn cylinder(
        name: impl Into<String>,
        around: Around,
        radius: f32,
        below: f32,
        above: f32,
    ) -> Self {
        Self::new(
            name,
            around,
            Shape::Cylinder {
                radius,
                below,
                above,
            },
        )
    }

    fn new(name: impl Into<String>, around: Around, shape: Shape) -> Self {
        Self {
            name: name.into(),
            around,
            shape,
            margin: ZONE_MARGIN,
            dwell: Duration::ZERO,
        }
    }

    /// This zone, which someone within it leaves only once they are `margin` beyond its radius, and its band.
    pub fn leaving_beyond(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    /// This zone, whose change from nobody within it to somebody, or back, is told only once it has lasted `dwell`, so
    /// that someone passing by, or stepping out for a moment, is not told.
    pub fn dwelling(mut self, dwell: Duration) -> Self {
        self.dwell = dwell;
        self
    }

    /// Its name, which its occupancy is told by.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it is around.
    pub fn around(&self) -> &Around {
        &self.around
    }

    /// Its shape.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// How far beyond its radius, and its band, someone within it goes before they leave it.
    pub fn margin(&self) -> f32 {
        self.margin
    }

    /// How long a change between nobody and somebody within it lasts before it is told.
    pub fn dwell(&self) -> Duration {
        self.dwell
    }

    /// Whether a head at `head` is within this zone about `centre`: entering it at its radius and band, or, for
    /// someone already `within` it, staying until they are beyond its margin. The server decides by this, in the
    /// space's coordinates, as an anchor's proximity is decided, across the floor for a cylinder.
    ///
    /// ```
    /// use tomori_environment::{Around, Zone};
    ///
    /// let zone = Zone::sphere("well", Around::point([0.0; 3]), 1.0);
    /// assert!(zone.holds([0.0; 3], [0.0, 0.0, 1.0], false));
    /// assert!(!zone.holds([0.0; 3], [0.0, 0.0, 1.2], false));
    /// assert!(zone.holds([0.0; 3], [0.0, 0.0, 1.2], true));
    /// assert!(!zone.holds([0.0; 3], [0.0, 0.0, 1.6], true));
    /// ```
    pub fn holds(&self, centre: [f32; 3], head: [f32; 3], within: bool) -> bool {
        let margin = if within { self.margin } else { 0.0 };
        let (x, y, z) = (
            head[0] - centre[0],
            head[1] - centre[1],
            head[2] - centre[2],
        );
        match self.shape {
            Shape::Sphere { radius } => (x * x + y * y + z * z).sqrt() <= radius + margin,
            Shape::Cylinder {
                radius,
                below,
                above,
            } => x.hypot(z) <= radius + margin && y >= -below - margin && y <= above + margin,
        }
    }
}

/// Why a declaration of zones is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneError(String);

impl fmt::Display for ZoneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<ZoneError> for crate::Error {
    fn from(error: ZoneError) -> Self {
        crate::Error::new(error.0)
    }
}

/// Whether the server takes `zones` as a declaration, or why not: at most [`ZONE_LIMIT`], each named as
/// [`crate::valid_reference`] takes, unlike the others, with a finite radius from [`LEAST_ZONE_RADIUS`] to
/// [`MOST_ZONE_RADIUS`], a cylinder's band from 0 to [`MOST_ZONE_HEIGHT`] below and above, of some height, a margin
/// from 0 to [`MOST_ZONE_MARGIN`], a dwell of at most [`MOST_ZONE_DWELL`], and what it is around a finite point, or an
/// anchor's name or an object's identity of 1 to [`crate::IDENTITY_LIMIT`] bytes. A side's tests may call it.
///
/// ```
/// use tomori_environment::{Around, Zone, check_zones};
///
/// assert!(check_zones(&[Zone::sphere("coop", Around::point([0.0; 3]), 2.0)]).is_ok());
/// assert!(check_zones(&[Zone::sphere("coop", Around::point([0.0; 3]), 0.1)]).is_err());
/// ```
pub fn check_zones(zones: &[Zone]) -> Result<(), ZoneError> {
    if zones.len() > ZONE_LIMIT {
        return Err(ZoneError(format!(
            "The environment declares {} zones, more than the {ZONE_LIMIT} the server takes.",
            zones.len()
        )));
    }
    for (index, zone) in zones.iter().enumerate() {
        let refused = |why: String| Err(ZoneError(format!("The zone {:?} {why}", zone.name)));
        if !crate::valid_reference(&zone.name) {
            return Err(ZoneError(format!(
                "A zone's name must be 1 to {ZONE_NAME_BYTES} bytes of ASCII letters, digits, '-', '_', '.' and '/'."
            )));
        }
        if zones[..index].iter().any(|other| other.name == zone.name) {
            return refused("is declared twice.".to_owned());
        }
        let radius = match zone.shape {
            Shape::Sphere { radius } => radius,
            Shape::Cylinder {
                radius,
                below,
                above,
            } => {
                let band = 0.0..=MOST_ZONE_HEIGHT;
                if !band.contains(&below) || !band.contains(&above) || below + above <= 0.0 {
                    return refused(format!(
                        "must reach from 0 to {MOST_ZONE_HEIGHT} m below and above its point, and have some height."
                    ));
                }
                radius
            }
        };
        if !(LEAST_ZONE_RADIUS..=MOST_ZONE_RADIUS).contains(&radius) {
            return refused(format!(
                "must have a radius from {LEAST_ZONE_RADIUS} to {MOST_ZONE_RADIUS} m."
            ));
        }
        if !(0.0..=MOST_ZONE_MARGIN).contains(&zone.margin) {
            return refused(format!(
                "must keep a margin from 0 to {MOST_ZONE_MARGIN} m."
            ));
        }
        if zone.dwell > MOST_ZONE_DWELL {
            return refused(format!(
                "must dwell at most {} s.",
                MOST_ZONE_DWELL.as_secs()
            ));
        }
        match &zone.around {
            Around::Point(point) if !point.iter().all(|value| value.is_finite()) => {
                return refused("must be around a finite point.".to_owned());
            }
            Around::Anchor(name) | Around::Object(name)
                if name.is_empty() || name.len() > crate::IDENTITY_LIMIT =>
            {
                return refused(format!(
                    "must be around an anchor or object named by 1 to {} bytes.",
                    crate::IDENTITY_LIMIT
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// How many participants are within one of the side's zones, as the server tells it: as the zone is declared, and as
/// that changes. Who they are it is not told: see `design/security.md`.
///
/// ```
/// use tomori_environment::Occupancy;
///
/// let occupancy = Occupancy::new("coop", 2);
/// assert_eq!((occupancy.zone(), occupancy.count(), occupancy.occupied()), ("coop", 2, true));
/// assert!(!Occupancy::new("coop", 0).occupied());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Occupancy {
    zone: String,
    count: u32,
}

impl Occupancy {
    /// `count` participants within the zone `zone`.
    pub fn new(zone: impl Into<String>, count: u32) -> Self {
        Self {
            zone: zone.into(),
            count,
        }
    }

    /// The zone's name, as the side declared it.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// How many participants are within it.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Whether anyone is within it.
    pub fn occupied(&self) -> bool {
        self.count > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(shape: Shape) -> Zone {
        Zone::new("a", Around::point([0.0; 3]), shape)
    }

    #[test]
    fn a_zone_is_what_it_was_given() {
        let zone = Zone::sphere("coop", Around::object("chicken-1"), 2.0);
        assert_eq!(zone.around(), &Around::Object("chicken-1".to_owned()));
        assert_eq!(zone.shape(), Shape::Sphere { radius: 2.0 });
        assert_eq!((zone.margin(), zone.dwell()), (ZONE_MARGIN, Duration::ZERO));
        assert_eq!(Around::anchor("a"), Around::Anchor("a".to_owned()));
        assert_eq!(Around::point([1.0; 3]), Around::Point([1.0; 3]));
        let occupancy = Occupancy::new("coop", 2);
        assert_eq!((occupancy.zone(), occupancy.count()), ("coop", 2));
        assert!(occupancy.occupied() && !Occupancy::new("coop", 0).occupied());
    }

    #[test]
    fn a_cylinder_holds_its_band_across_the_floor_and_its_margin_once_within() {
        let zone = zone(Shape::Cylinder {
            radius: 1.0,
            below: 0.5,
            above: 2.0,
        });
        let centre = [2.0, 1.0, 2.0];
        // Across the floor, whatever the height within the band.
        assert!(zone.holds(centre, [2.9, 2.9, 2.0], false));
        assert!(zone.holds(centre, [2.0, 0.5, 2.0], false));
        assert!(!zone.holds(centre, [2.0, 0.4, 2.0], false));
        assert!(!zone.holds(centre, [2.0, 3.1, 2.0], false));
        assert!(!zone.holds(centre, [3.1, 1.0, 2.0], false));
        // Once within, until beyond its margin, across and up and down.
        assert!(zone.holds(centre, [3.4, 1.0, 2.0], true));
        assert!(zone.holds(centre, [2.0, 0.1, 2.0], true));
        assert!(zone.holds(centre, [2.0, 3.4, 2.0], true));
        assert!(!zone.holds(centre, [3.6, 1.0, 2.0], true));
        assert!(!zone.holds(centre, [2.0, -0.1, 2.0], true));
    }

    #[test]
    fn a_declaration_is_taken_at_every_limit_and_refused_past_each() {
        let at_limits: Vec<Zone> = (0..ZONE_LIMIT)
            .map(|index| {
                Zone::cylinder(
                    format!("zone/{index}"),
                    Around::anchor("a".repeat(crate::IDENTITY_LIMIT)),
                    if index % 2 == 0 {
                        LEAST_ZONE_RADIUS
                    } else {
                        MOST_ZONE_RADIUS
                    },
                    MOST_ZONE_HEIGHT,
                    0.0,
                )
                .leaving_beyond(MOST_ZONE_MARGIN)
                .dwelling(MOST_ZONE_DWELL)
            })
            .collect();
        assert_eq!(check_zones(&at_limits), Ok(()));
        assert_eq!(
            check_zones(&[
                Zone::sphere("a".repeat(ZONE_NAME_BYTES), Around::object("x"), 1.0)
                    .leaving_beyond(0.0)
            ]),
            Ok(())
        );
        assert_eq!(check_zones(&[]), Ok(()));
        let mut many = at_limits.clone();
        many.push(Zone::sphere("more", Around::point([0.0; 3]), 1.0));
        let sphere = |radius| Zone::sphere("z", Around::point([0.0; 3]), radius);
        let cylinder =
            |below, above| Zone::cylinder("z", Around::point([0.0; 3]), 1.0, below, above);
        for (refused, why) in [
            (many, "17 zones, more than the 16"),
            (vec![sphere(1.0), sphere(2.0)], "declared twice"),
            (
                vec![Zone::sphere("a b", Around::point([0.0; 3]), 1.0)],
                "A zone's name",
            ),
            (
                vec![Zone::sphere(
                    "a".repeat(ZONE_NAME_BYTES + 1),
                    Around::point([0.0; 3]),
                    1.0,
                )],
                "A zone's name",
            ),
            (vec![sphere(0.29)], "radius from 0.3 to 20"),
            (vec![sphere(20.01)], "radius"),
            (vec![sphere(f32::NAN)], "radius"),
            (vec![cylinder(0.0, 0.0)], "some height"),
            (vec![cylinder(-0.1, 1.0)], "some height"),
            (vec![cylinder(0.0, MOST_ZONE_HEIGHT + 0.1)], "some height"),
            (vec![cylinder(f32::NAN, 1.0)], "some height"),
            (vec![sphere(1.0).leaving_beyond(-0.1)], "margin"),
            (vec![sphere(1.0).leaving_beyond(5.1)], "margin"),
            (
                vec![sphere(1.0).dwelling(MOST_ZONE_DWELL + Duration::from_millis(1))],
                "dwell at most 60 s",
            ),
            (
                vec![Zone::sphere(
                    "z",
                    Around::point([0.0, f32::INFINITY, 0.0]),
                    1.0,
                )],
                "finite point",
            ),
            (
                vec![Zone::sphere("z", Around::anchor(""), 1.0)],
                "anchor or object",
            ),
            (
                vec![Zone::sphere(
                    "z",
                    Around::object("a".repeat(crate::IDENTITY_LIMIT + 1)),
                    1.0,
                )],
                "anchor or object",
            ),
        ] {
            let error = check_zones(&refused).unwrap_err();
            assert!(error.to_string().contains(why), "{error} for {why}");
            assert_eq!(crate::Error::from(error.clone()).message(), error.0);
        }
    }

    /// The limits a side keeps within are those `design/environments.md` states, so that changing one is deliberate.
    #[test]
    fn the_limits_of_zones_and_presence_are_those_the_design_states() {
        assert_eq!((ZONE_LIMIT, ZONE_NAME_BYTES), (16, 64));
        assert_eq!((LEAST_ZONE_RADIUS, MOST_ZONE_RADIUS), (0.3, 20.0));
        assert_eq!(
            (ZONE_MARGIN, MOST_ZONE_MARGIN, MOST_ZONE_HEIGHT),
            (0.5, 5.0, 20.0)
        );
        assert_eq!(MOST_ZONE_DWELL, Duration::from_secs(60));
        assert_eq!(ZONE_INTERVAL, Duration::from_millis(900));
        assert_eq!(PRESENCE_LIMIT, 64);
        assert_eq!(PRESENCE_STALE, Duration::from_secs(5));
    }
}
