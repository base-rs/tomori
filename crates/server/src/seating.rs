// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where whatever stands at a space's cluster anchors stands: the slot the server gives each member of each cluster, in
//! order of arrival, and the span each cluster is laid out for, kept a slot higher for a while as members leave, which
//! the space's state sends so that every client lays each cluster out alike. A member keeps its slot while it stays;
//! gaps close only as the cluster passes a step, when everyone moves anyway, and a member gathered into the "+N more"
//! sign takes a slot that frees, as `tomori_protocol::cluster::Cluster::reseat` says. See `design/environments.md`.
use std::collections::BTreeMap;

use tomori_protocol::cluster::{Anchor, Cluster, Layout};

/// What stands in a cluster: a worker, by its session; a floating panel, by its identity; or the agent.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Member {
    Worker(String),
    Panel(String),
    Agent,
}

/// One cluster's members, in order of arrival, each with its slot, and the span it is laid out for.
#[derive(Clone, Debug, Default, PartialEq)]
struct Seated {
    seats: Vec<(Member, usize)>,
    laid: usize,
}

/// The members of a space's clusters, by anchor name. See the module's documentation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Seating {
    clusters: BTreeMap<String, Seated>,
}

impl Seating {
    /// Seat each of `wanted`, a member and the name of the anchor it stands at, in order of arrival, among `anchors`,
    /// the space's anchors now: those already seated there keep their slots, those that went, or moved to another, are
    /// taken out, and those new take the lowest free slot there; then each cluster's gaps close as
    /// `Cluster::reseat` closes them, and it is laid out again for the span that leaves. A member at an anchor not among
    /// `anchors` stands nowhere. Returns whether any member's slot, or any cluster's span, changed.
    pub(crate) fn seat(&mut self, anchors: &[Anchor], wanted: &[(Member, &str)]) -> bool {
        let before = self.clone();
        // Those no longer there, or there no longer, leave.
        self.clusters.retain(|name, seated| {
            seated.seats.retain(|(member, _)| {
                wanted
                    .iter()
                    .any(|(wanting, at)| wanting == member && *at == name.as_str())
            });
            anchors.iter().any(|anchor| &anchor.name == name)
        });
        for (member, at) in wanted {
            if !anchors.iter().any(|anchor| anchor.name == *at) {
                continue;
            }
            let seated = self.clusters.entry((*at).to_owned()).or_default();
            if seated.seats.iter().any(|(seated, _)| seated == member) {
                continue;
            }
            let free = (0..)
                .find(|slot| !seated.seats.iter().any(|(_, taken)| taken == slot))
                .expect("a free slot");
            seated.seats.push((member.clone(), free));
        }
        for anchor in anchors {
            let Some(seated) = self.clusters.get_mut(&anchor.name) else {
                continue;
            };
            let mut slots: Vec<usize> = seated.seats.iter().map(|(_, slot)| *slot).collect();
            seated.laid = Cluster::new(anchor).reseat(&mut slots, &[], seated.laid);
            for ((_, slot), reseated) in seated.seats.iter_mut().zip(slots) {
                *slot = reseated;
            }
        }
        self.clusters.retain(|_, seated| !seated.seats.is_empty());
        *self != before
    }

    /// The anchor `member` stands at, by name, and its slot there, if it stands at one.
    pub(crate) fn seat_of(&self, member: &Member) -> Option<(&str, usize)> {
        self.clusters.iter().find_map(|(name, seated)| {
            seated
                .seats
                .iter()
                .find(|(seated, _)| seated == member)
                .map(|(_, slot)| (name.as_str(), *slot))
        })
    }

    /// Where `member` stands, in the space, as its cluster among `anchors` lays it out, and the eyes of whoever that
    /// cluster gathers about, if it stands at one.
    pub(crate) fn position(
        &self,
        anchors: &[Anchor],
        member: &Member,
    ) -> Option<([f32; 3], [f32; 3])> {
        let (name, slot) = self.seat_of(member)?;
        let anchor = anchors.iter().find(|anchor| anchor.name == name)?;
        let cluster = Cluster::new(anchor);
        let at = cluster.slot(&cluster.arrange(self.laid(name)), slot);
        Some((at.position, cluster.audience()))
    }

    /// Where a member would stand at the anchor `anchor`, arriving there now among those already seated, after `before`
    /// others arriving with it: in the lowest free slot left, as [`Self::seat`] would seat it, in the cluster laid out
    /// for it; and the eyes of whoever it gathers about.
    pub(crate) fn arriving(&self, anchor: &Anchor, before: usize) -> ([f32; 3], [f32; 3]) {
        let taken = self.members(&anchor.name);
        let free = (0..)
            .filter(|slot| !taken.iter().any(|(_, taken)| taken == slot))
            .nth(before)
            .expect("a free slot");
        let cluster = Cluster::new(anchor);
        let laid = Cluster::settle(self.laid(&anchor.name), free + 1);
        let at = cluster.slot(&cluster.arrange(laid), free);
        (at.position, cluster.audience())
    }

    /// The members standing at the anchor `name`, in order of arrival, each with its slot.
    pub(crate) fn members(&self, name: &str) -> &[(Member, usize)] {
        self.clusters
            .get(name)
            .map_or(&[], |seated| seated.seats.as_slice())
    }

    /// The span the cluster at the anchor `name` is laid out for.
    pub(crate) fn laid(&self, name: &str) -> usize {
        self.clusters.get(name).map_or(0, |seated| seated.laid)
    }

    /// The members of `anchor`'s cluster gathered into its "+N more" sign, in the order of their slots.
    pub(crate) fn gathered(&self, anchor: &Anchor) -> Vec<&Member> {
        let Some(more) = Cluster::new(anchor).arrange(self.laid(&anchor.name)).more() else {
            return Vec::new();
        };
        let mut gathered: Vec<&(Member, usize)> = self
            .members(&anchor.name)
            .iter()
            .filter(|(_, slot)| *slot >= more)
            .collect();
        gathered.sort_by_key(|(_, slot)| *slot);
        gathered.into_iter().map(|(member, _)| member).collect()
    }

    /// How each cluster that a worker stands in is laid out, by anchor name, as the space's state sends it: the span it
    /// is laid out for, and each worker's slot, in the order of their slots.
    pub(crate) fn layouts(&self) -> Vec<Layout> {
        self.clusters
            .iter()
            .map(|(name, seated)| {
                let mut workers: Vec<(&str, usize)> = seated
                    .seats
                    .iter()
                    .filter_map(|(member, slot)| match member {
                        Member::Worker(session) => Some((session.as_str(), *slot)),
                        _ => None,
                    })
                    .collect();
                workers.sort_by_key(|(_, slot)| *slot);
                workers.into_iter().fold(
                    Layout::new(name.as_str(), seated.laid as u32),
                    |layout, (session, slot)| layout.with_seat(session, slot as u32),
                )
            })
            .collect()
    }
}

/// The `home` anchor among `anchors`, as a space's state sends them: about the workers anchor, or else the agent's
/// home, in the room its environment gives it, if it gives any, as every client lays the home cluster out.
pub(crate) fn home(anchors: &tomori_protocol::Anchors) -> Anchor {
    let pose = anchors
        .workers
        .unwrap_or_else(|| crate::hub::agent_home(anchors));
    let mut home = Anchor::new(tomori_protocol::cluster::HOME, pose);
    home.extent = anchors
        .cluster(tomori_protocol::cluster::HOME)
        .and_then(|home| home.extent);
    home
}

/// Where each of `workers`, the space's, oldest first, stands, as [`Seating::seat`] takes it: at the anchor `placed`
/// says it was placed at, by session, where that is among `anchors`; or else with the worker that delegated to it,
/// wherever that stands, since it works on its task; or else at `home`.
pub(crate) fn workers<'p>(
    workers: &[tomori_protocol::Worker],
    anchors: &[Anchor],
    placed: impl Fn(&str) -> Option<&'p str>,
) -> Vec<(Member, &'p str)> {
    let here = |session: &str| {
        placed(session).filter(|at| anchors.iter().any(|anchor| anchor.name == *at))
    };
    workers
        .iter()
        .map(|worker| {
            // Up its parents, as far as there are workers, so that a cycle ends.
            let mut at = here(&worker.id);
            let mut parent = worker.parent.as_deref();
            for _ in 0..workers.len() {
                let Some(above) = parent.filter(|_| at.is_none()) else {
                    break;
                };
                at = here(above);
                parent = workers
                    .iter()
                    .find(|known| known.id == above)
                    .and_then(|known| known.parent.as_deref());
            }
            (
                Member::Worker(worker.id.clone()),
                at.unwrap_or(tomori_protocol::cluster::HOME),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests;
