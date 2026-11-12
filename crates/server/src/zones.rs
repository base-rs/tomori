// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who is within the proximity zones an environment's server side declares: as the hub sweeps, each participant whose
//! head, as their client last reported it, is within a zone's radius enters it, and stays until they are beyond its
//! margin; each zone's occupancy is told to the side as the zone is declared, whatever it is, and as how many are within
//! it changes, a change between nobody and somebody only once it has lasted the zone's dwell. Pure: the time, the heads,
//! where each zone stands and how to tell the side are given; looking allocates nothing. See `design/environments.md`.
use std::time::Instant;

use tomori_environment::{Around, Occupancy, PRESENCE_LIMIT, ZONE_INTERVAL, Zone};

/// A participant's head, by their number, in the space's coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Head {
    pub participant: u32,
    pub at: [f32; 3],
}

/// Where the anchors and objects zones may be around stand now, in the space's coordinates, by name or identity.
pub struct Centres<'a> {
    pub anchor: &'a dyn Fn(&str) -> Option<[f32; 3]>,
    pub object: &'a dyn Fn(&str) -> Option<[f32; 3]>,
}

impl Centres<'_> {
    /// Where nothing stands.
    #[cfg(test)]
    pub fn nowhere() -> Centres<'static> {
        Centres {
            anchor: &|_| None,
            object: &|_| None,
        }
    }
}

/// One zone, as the server keeps it.
#[derive(Debug)]
struct Kept {
    zone: Zone,
    /// Its point, carried into the space's coordinates, for one around a point.
    point: Option<[f32; 3]>,
    /// Who is within it, by number, as last looked at.
    members: Vec<u32>,
    /// How many the side was last told are within it, or none until it is told.
    told: Option<u32>,
    /// Since when it has been occupied, or not, otherwise than the side was told, while that waits for its dwell.
    since: Option<Instant>,
}

/// The zones a side declared for its binding, and who is within each.
#[derive(Debug, Default)]
pub struct Zones {
    zones: Vec<Kept>,
    /// Where who is within a zone is gathered as it is looked at, swapped with its members.
    scratch: Vec<u32>,
}

/// What a side's binding declared of zones: those it looks at, a declaration that came sooner than [`ZONE_INTERVAL`]
/// after the last taken, waiting, the latest replacing the one before, and the heads the hub gathers, kept so that
/// sweeping allocates them once.
#[derive(Debug, Default)]
pub struct Declared {
    current: Option<Zones>,
    pending: Option<Zones>,
    taken: Option<Instant>,
    pub heads: Vec<Head>,
}

impl Declared {
    /// Take `zones` at `now` in place of those declared before, carrying who is within each zone that keeps its name,
    /// or, sooner than [`ZONE_INTERVAL`] after the last taken, let them wait until [`Declared::due`] may take them.
    pub fn declare(&mut self, zones: Zones, now: Instant) {
        self.pending = Some(zones);
        self.due(now);
    }

    /// Take the declaration waiting, if there is one and [`ZONE_INTERVAL`] has passed since the last taken.
    pub fn due(&mut self, now: Instant) {
        let ready = self
            .taken
            .is_none_or(|taken| now.saturating_duration_since(taken) >= ZONE_INTERVAL);
        if ready && let Some(mut zones) = self.pending.take() {
            if let Some(previous) = &self.current {
                zones.carry(previous);
            }
            self.current = Some(zones);
            self.taken = Some(now);
        }
    }

    /// Whether any zones were declared, taken or waiting.
    pub fn any(&self) -> bool {
        self.current.is_some() || self.pending.is_some()
    }

    /// The zones taken, if any.
    pub fn current(&mut self) -> Option<&mut Zones> {
        self.current.as_mut()
    }
}

impl Zones {
    /// The zones `zones`, each around a point carried into the space's coordinates by `carry`, or why they are refused:
    /// one [`tomori_environment::check_zones`] refuses, or around a point `carry` does not take. None is told until the
    /// hub next looks, when each is told.
    pub fn declare(
        zones: Vec<Zone>,
        carry: impl Fn([f32; 3]) -> Option<[f32; 3]>,
    ) -> Result<Self, String> {
        tomori_environment::check_zones(&zones).map_err(|error| error.to_string())?;
        let zones = zones
            .into_iter()
            .map(|zone| {
                let point = match zone.around() {
                    Around::Point(point) => Some(carry(*point).ok_or_else(|| {
                        format!(
                            "The zone {:?} is around a point more than 30 m from home.",
                            zone.name()
                        )
                    })?),
                    _ => None,
                };
                Ok(Kept {
                    zone,
                    point,
                    members: Vec::with_capacity(PRESENCE_LIMIT),
                    told: None,
                    since: None,
                })
            })
            .collect::<Result<Vec<Kept>, String>>()?;
        Ok(Self {
            zones,
            scratch: Vec::with_capacity(PRESENCE_LIMIT),
        })
    }

    /// Carry who is within each zone of `previous` into the zone of the same name here, so that someone within its
    /// margin, but not its radius, stays within it as it is declared again; each is still told as it is at the next look.
    fn carry(&mut self, previous: &Zones) {
        for kept in &mut self.zones {
            if let Some(before) = previous
                .zones
                .iter()
                .find(|before| before.zone.name() == kept.zone.name())
            {
                kept.members.clear();
                kept.members.extend(&before.members);
            }
        }
    }

    /// Whether any zone is around an anchor, which the hub then finds.
    pub fn around_anchors(&self) -> bool {
        self.zones
            .iter()
            .any(|kept| matches!(kept.zone.around(), Around::Anchor(_)))
    }

    /// Look at `heads` at `now`, the first [`PRESENCE_LIMIT`] of them, within each zone about where `centres` say the
    /// anchor or object it is around stands now, nobody within one whose centre is not there; and `tell` the side each zone's occupancy
    /// as it is first looked at, and as it changes, a change between nobody and somebody only once it has lasted the
    /// zone's dwell. One `tell` cannot give, as the side's inbox is full, is told at a later look.
    pub fn observe(
        &mut self,
        now: Instant,
        heads: &[Head],
        centres: Centres<'_>,
        mut tell: impl FnMut(Occupancy) -> bool,
    ) {
        let heads = &heads[..heads.len().min(PRESENCE_LIMIT)];
        for kept in &mut self.zones {
            let centre = match kept.zone.around() {
                Around::Point(_) => kept.point,
                Around::Anchor(name) => (centres.anchor)(name),
                Around::Object(id) => (centres.object)(id),
            };
            self.scratch.clear();
            if let Some(centre) = centre {
                for head in heads {
                    let within = kept.members.contains(&head.participant);
                    if kept.zone.holds(centre, head.at, within)
                        && !self.scratch.contains(&head.participant)
                    {
                        self.scratch.push(head.participant);
                    }
                }
            }
            std::mem::swap(&mut kept.members, &mut self.scratch);
            let count = kept.members.len() as u32;
            let due = match kept.told {
                None => true,
                Some(told) if told == count => {
                    kept.since = None;
                    false
                }
                Some(told) if (told > 0) == (count > 0) => true,
                Some(_) => {
                    let since = *kept.since.get_or_insert(now);
                    now.saturating_duration_since(since) >= kept.zone.dwell()
                }
            };
            if due && tell(Occupancy::new(kept.zone.name(), count)) {
                kept.told = Some(count);
                kept.since = None;
            }
        }
    }

    /// How many are within the zone `name`, as last looked at, if there is one.
    #[cfg(test)]
    pub fn count(&self, name: &str) -> Option<usize> {
        self.zones
            .iter()
            .find(|kept| kept.zone.name() == name)
            .map(|kept| kept.members.len())
    }
}

#[cfg(test)]
mod tests;
