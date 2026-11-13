// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where people arrive as they join a space: the server chooses one of its environment's spawn points for each, by
//! `tomori_protocol::spawn::choose`, from what it alone knows, their role and where everyone in the space stands, and
//! serialises their arrivals, so that two joining at once arrive at different points. It holds no I/O: the hub asks it
//! as someone joins and sends them what it chose. See `design/environments.md`.
use std::collections::BTreeMap;

use tomori_protocol::Pose;
use tomori_protocol::spawn::{self, Spawn};

/// What a space keeps of its arrivals, for its current environment and this run only.
#[derive(Debug, Default)]
pub(crate) struct Arrivals {
    /// How many have arrived at each of the environment's spawn points, by its place among them, so that arrivals go
    /// round them.
    given: Vec<u32>,
    /// Where each arrived, by their key: kept while they stay in the space, as they reconnect to it too, so that a
    /// reconnect arrives where they arrived and counts nothing, until they leave it, join another or it changes
    /// environment; and where they are taken to stand until they report where they are, as a visitor never does.
    arrived: BTreeMap<String, Pose>,
}

impl Arrivals {
    /// Where `who` arrived, if they have not left since: a reconnect arrives there again.
    pub(crate) fn kept(&self, who: &str) -> Option<Pose> {
        self.arrived.get(who).copied()
    }

    /// Where `who`, arriving as `tag`, arrives among `spawns`, in the space's coordinates, with `others` in the space,
    /// each by their key and their head where they have lately reported it; or none, at home, where no point suits
    /// them. Those who have not reported stand where they arrived.
    pub(crate) fn choose(
        &mut self,
        who: &str,
        tag: &str,
        spawns: &[Spawn],
        others: &[(String, Option<[f32; 3]>)],
    ) -> Option<Pose> {
        let standing: Vec<[f32; 3]> = others
            .iter()
            .filter_map(|(key, head)| {
                head.or_else(|| {
                    let [x, y, z] = self.arrived.get(key)?.position;
                    Some([x, y + spawn::EYES, z])
                })
            })
            .collect();
        let index = spawn::choose(spawns, tag, &standing, &self.given)?;
        if self.given.len() <= index {
            self.given.resize(index + 1, 0);
        }
        self.given[index] += 1;
        let pose = spawns[index].pose;
        self.arrived.insert(who.to_owned(), pose);
        Some(pose)
    }

    /// `who` has left the space, or joined another: they arrive anew when they next join it.
    pub(crate) fn forget(&mut self, who: &str) {
        self.arrived.remove(who);
    }

    /// Keep only those `remembered` says the server still keeps in the space, as those away reconnect to it.
    pub(crate) fn retain(&mut self, remembered: impl Fn(&str) -> bool) {
        self.arrived.retain(|key, _| remembered(key));
    }

    /// `from` is known as `to` from now on, as a visitor admitted is, where they arrived.
    pub(crate) fn rekey(&mut self, from: &str, to: &str) {
        if let Some(pose) = self.arrived.remove(from) {
            self.arrived.insert(to.to_owned(), pose);
        }
    }

    /// The space has changed environment: everyone arrives anew among its points.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawns() -> Vec<Spawn> {
        vec![
            Spawn::new(Pose::at([0.0, 0.0, 0.0])),
            Spawn::new(Pose::at([3.0, 0.0, 0.0])),
            Spawn::new(Pose::at([0.0, 0.0, 5.0])).with_tag(spawn::VISITOR),
        ]
    }

    #[test]
    fn people_joining_at_once_arrive_apart() {
        let spawns = spawns();
        let mut arrivals = Arrivals::default();
        let ada = arrivals.choose("ada", spawn::OWNER, &spawns, &[]);
        assert_eq!(ada.map(|pose| pose.position), Some([0.0, 0.0, 0.0]));
        // Ada has not reported where she is, so she stands where she arrived.
        let others = [("ada".to_owned(), None)];
        let bob = arrivals.choose("bob", spawn::COLLABORATOR, &spawns, &others);
        assert_eq!(bob.map(|pose| pose.position), Some([3.0, 0.0, 0.0]));
        // A visitor arrives by the gate, tagged for them.
        let others = [("ada".to_owned(), None), ("bob".to_owned(), None)];
        let eve = arrivals.choose("eve", spawn::VISITOR, &spawns, &others);
        assert_eq!(eve.map(|pose| pose.position), Some([0.0, 0.0, 5.0]));
    }

    #[test]
    fn where_people_stand_now_counts_and_those_gone_stand_nowhere() {
        let spawns = spawns();
        let mut arrivals = Arrivals::default();
        arrivals.choose("ada", spawn::OWNER, &spawns, &[]);
        // Ada walked over to the second point, so the first is free, though more have arrived there.
        let others = [("ada".to_owned(), Some([3.2, 1.6, 0.0]))];
        let bob = arrivals.choose("bob", spawn::OWNER, &spawns, &others);
        assert_eq!(bob.map(|pose| pose.position), Some([0.0, 0.0, 0.0]));
        // Both left: the next goes round to the point fewest arrived at.
        let carol = arrivals.choose("carol", spawn::OWNER, &spawns, &[]);
        assert_eq!(carol.map(|pose| pose.position), Some([3.0, 0.0, 0.0]));
        // Each is kept where they arrived until they leave, or the server forgets them.
        assert_eq!(
            arrivals.kept("bob").map(|pose| pose.position),
            Some([0.0, 0.0, 0.0])
        );
        arrivals.forget("bob");
        assert_eq!(arrivals.kept("bob"), None);
        arrivals.retain(|key| key == "carol");
        assert_eq!(arrivals.arrived.keys().collect::<Vec<_>>(), ["carol"]);
        // Admitted, as a collaborator's key, Carol is kept where she arrived; anyone else is nobody to move.
        arrivals.rekey("carol", "1:carol/carol");
        arrivals.rekey("nobody", "1:nobody/nobody");
        assert_eq!(
            arrivals.kept("1:carol/carol").map(|pose| pose.position),
            Some([3.0, 0.0, 0.0])
        );
        assert_eq!(arrivals.arrived.len(), 1);
    }

    #[test]
    fn nobody_suited_arrives_at_home_and_a_new_environment_starts_again() {
        let only = [Spawn::new(Pose::HOME).with_tag(spawn::VISITOR)];
        let mut arrivals = Arrivals::default();
        assert_eq!(arrivals.choose("ada", spawn::OWNER, &only, &[]), None);
        assert_eq!(arrivals.choose("ada", spawn::OWNER, &[], &[]), None);
        assert!(arrivals.choose("eve", spawn::VISITOR, &only, &[]).is_some());
        arrivals.reset();
        assert!(arrivals.given.is_empty() && arrivals.arrived.is_empty());
    }
}
