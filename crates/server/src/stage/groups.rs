// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Walls snapped together into groups. A group is the walls someone added in one environment of a space that are
//! attached to each other, each but its first across an edge of another, aligned along it, kept with the space as that
//! attachment, so that a group stays joined as its walls change: where each stands follows from its first wall's pose, as
//! `tomori_protocol::group` lays it out. Carrying any of its walls carries the group; one taken out leaves the rest
//! together where they still touch. No two walls of a group overlap, and all are flat, or all curved at one radius,
//! about one line, each taking the radius of the wall it is attached to. See `design/projection.md`.
use std::collections::BTreeMap;

use tomori_protocol::{Align, Attach, Edge, Join, Pose, Wall, WallKind, group};

use super::{AddedWall, Layout, Room, Stage, Walls, valid};

/// How near a wall must stand to where an attachment would put it, in metres, to be taken as attached so.
const STANDS: f32 = 0.002;

/// A wall someone added, as it is shown.
fn shown(added: &AddedWall) -> Wall {
    added.layout.unwrap_or(Layout::FLAT).apply(Wall::new(
        added.id.clone(),
        WallKind::Added,
        added.pose,
    ))
}

/// Whether `a` and `b` stand within [`STANDS`] of each other, facing alike.
fn stands_at(a: &Pose, b: &Pose) -> bool {
    let apart = (0..3)
        .map(|axis| (a.position[axis] - b.position[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    let alike = a
        .orientation
        .iter()
        .zip(b.orientation)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .abs();
    apart <= STANDS && alike >= 1.0 - 1e-5
}

impl Room {
    fn added(&self, wall: &str) -> Option<&AddedWall> {
        self.walls.iter().find(|added| added.id == wall)
    }

    fn added_mut(&mut self, wall: &str) -> Option<&mut AddedWall> {
        self.walls.iter_mut().find(|added| added.id == wall)
    }

    /// The identities of the walls of `wall`'s group, itself first, the others in the order they were added: those it is
    /// attached to, or attached to it, through each other.
    pub(super) fn group(&self, wall: &str) -> Vec<String> {
        let mut found = vec![wall.to_owned()];
        let mut index = 0;
        while index < found.len() {
            let at = found[index].clone();
            for added in &self.walls {
                let linked = added.attach.as_ref().is_some_and(|attach| attach.to == at)
                    || self
                        .added(&at)
                        .and_then(|wall| wall.attach.as_ref())
                        .is_some_and(|attach| attach.to == added.id);
                if linked && !found.contains(&added.id) {
                    found.push(added.id.clone());
                }
            }
            index += 1;
        }
        found
    }

    /// The first wall of `wall`'s group, attached to none, which the rest stand from.
    fn root(&self, wall: &str) -> String {
        let mut at = wall.to_owned();
        for _ in 0..self.walls.len() {
            match self.added(&at).and_then(|added| added.attach.as_ref()) {
                Some(attach) if self.added(&attach.to).is_some() => at = attach.to.clone(),
                _ => break,
            }
        }
        at
    }

    /// Make `wall` its group's first: each attachment between it and the first is turned around, so that every wall
    /// stands where it stood.
    fn reroot(&mut self, wall: &str) {
        let mut path = vec![wall.to_owned()];
        let mut attachments = Vec::new();
        while let Some(attach) = self
            .added(path.last().expect("a path"))
            .and_then(|added| added.attach.clone())
            .filter(|attach| {
                self.added(&attach.to).is_some()
                    && !path.contains(&attach.to)
                    && path.len() <= self.walls.len()
            })
        {
            path.push(attach.to.clone());
            attachments.push(attach);
        }
        self.added_mut(wall).expect("a wall here").attach = None;
        for (index, attach) in attachments.into_iter().enumerate() {
            let turned = Attach::new(path[index].clone(), attach.edge.opposite(), attach.align);
            self.added_mut(&path[index + 1])
                .expect("a wall attached to is here")
                .attach = Some(turned);
        }
    }

    /// The attachment that puts `wall` where it stands, beside one of `candidates`, if one does.
    fn found_attached(&self, wall: &AddedWall, candidates: &[String]) -> Option<Attach> {
        let wall = shown(wall);
        candidates
            .iter()
            .filter_map(|candidate| self.added(candidate))
            .map(shown)
            .find_map(|other| {
                Edge::ALL.into_iter().find_map(|edge| {
                    Align::ALL.into_iter().find_map(|align| {
                        group::attached_pose(&other, &wall, edge, align)
                            .filter(|pose| stands_at(pose, &wall.pose))
                            .map(|_| Attach::new(other.id.clone(), edge, align))
                    })
                })
            })
    }

    /// Take `wall` alone out of its group, where it stands: the walls attached to it stand where they are, attached to
    /// another of the group they still touch as they were, or else as groups of their own.
    pub(super) fn detach(&mut self, wall: &str) {
        let members = self.group(wall);
        let children: Vec<String> = self
            .walls
            .iter()
            .filter(|added| {
                added
                    .attach
                    .as_ref()
                    .is_some_and(|attach| attach.to == wall)
            })
            .map(|added| added.id.clone())
            .collect();
        self.added_mut(wall).expect("a wall here").attach = None;
        for child in children {
            self.added_mut(&child).expect("a wall here").attach = None;
            // Of the rest of the group, those not now its own.
            let own = self.group(&child);
            let others: Vec<String> = members
                .iter()
                .filter(|member| *member != wall && !own.contains(member))
                .cloned()
                .collect();
            let attach = self.found_attached(self.added(&child).expect("a wall here"), &others);
            self.added_mut(&child).expect("a wall here").attach = attach;
        }
    }

    /// Carry `wall`'s group so that `wall` stands at `pose`, the rest standing where they stood from it.
    pub(super) fn carry(&mut self, wall: &str, pose: Pose) {
        let from = self.added(wall).expect("a wall here").pose;
        let root = self.root(wall);
        let first = self.added_mut(&root).expect("a group's first wall is here");
        first.pose = group::carried_with(&from, &pose, &first.pose);
    }

    /// Stand each wall attached to another where its attachment puts it, from its group's first wall; and say why not,
    /// leaving it as it is to be put back, where a group would have a wall beyond reach, walls of different shapes, or
    /// walls that overlap where any wall changed from `before`. An attachment to a wall not here, to an edge or with an
    /// alignment this server does not know, as a newer one may have kept, or one that would go round in a circle, is
    /// forgotten.
    pub(super) fn settle(&mut self, before: &Room) -> Result<(), &'static str> {
        for added in &mut self.walls {
            if added.attach.as_ref().is_some_and(|attach| !attach.known()) {
                added.attach = None;
            }
        }
        let ids: Vec<String> = self.walls.iter().map(|added| added.id.clone()).collect();
        for id in &ids {
            // The first wall reached is attached to nothing here only where it is attached to a wall not here, or the
            // attachments go round in a circle.
            let root = self.root(id);
            self.added_mut(&root).expect("a wall here").attach = None;
        }
        // From each first wall outwards, each stands across the edge of the one it is attached to.
        let mut placed: Vec<String> = ids
            .iter()
            .filter(|id| self.added(id).is_some_and(|added| added.attach.is_none()))
            .cloned()
            .collect();
        // A group's first wall, carried with another of its walls, stands within reach too.
        if placed
            .iter()
            .any(|root| valid(&self.added(root).expect("a wall here").pose).is_none())
        {
            return Err("A group's walls must stand within 30 m of the space's middle.");
        }
        let mut index = 0;
        while index < placed.len() {
            let parent = shown(self.added(&placed[index]).expect("a wall here"));
            let children: Vec<(String, Attach)> = self
                .walls
                .iter()
                .filter_map(|added| {
                    let attach = added.attach.clone()?;
                    (attach.to == parent.id).then(|| (added.id.clone(), attach))
                })
                .collect();
            for (child, attach) in children {
                let mut wall = shown(self.added(&child).expect("a wall here"));
                match (parent.radius(), wall.radius()) {
                    (None, None) => {}
                    // Curved, it takes the radius of the wall it is attached to, so that the group stands on one arc,
                    // about one line, whatever its walls' slots.
                    (Some(radius), Some(own)) => {
                        if own != radius {
                            let added = self.added_mut(&child).expect("a wall here");
                            let mut layout = added.layout.unwrap_or(Layout::FLAT);
                            layout.radius = Some(radius);
                            let bent =
                                layout.apply(Wall::new(child.clone(), WallKind::Added, added.pose));
                            Stage::fits(&bent, layout)?;
                            added.layout = Some(layout);
                            wall = bent;
                        }
                    }
                    _ => {
                        return Err("Walls snap together only when all are flat, or all curved.");
                    }
                }
                let pose = group::attached_pose(&parent, &wall, attach.edge, attach.align)
                    .and_then(|pose| valid(&pose))
                    .ok_or("A group's walls must stand within 30 m of the space's middle.")?;
                self.added_mut(&child).expect("a wall here").pose = pose;
                placed.push(child);
            }
            index += 1;
        }
        // A group's walls stand upright, so that it reads as one board: a wall snapped while tilted stands upright, as it
        // takes the radius of a curved wall.
        let grouped: Vec<String> = self
            .walls
            .iter()
            .filter_map(|added| added.attach.as_ref())
            .map(|attach| attach.to.clone())
            .chain(
                self.walls
                    .iter()
                    .filter(|added| added.attach.is_some())
                    .map(|added| added.id.clone()),
            )
            .collect();
        for added in self
            .walls
            .iter_mut()
            .filter(|added| grouped.contains(&added.id))
        {
            if let Some(layout) = added.layout.as_mut().filter(|layout| layout.tilt.is_some()) {
                layout.tilt = None;
                if *layout == Layout::FLAT {
                    added.layout = None;
                }
            }
        }
        // No two walls overlap, where either has changed.
        let shown_walls: Vec<Wall> = self.walls.iter().map(shown).collect();
        let changed = |wall: &Wall| {
            before.added(&wall.id).is_none_or(|was| {
                shown(was) != *wall
                    || was.attach != self.added(&wall.id).and_then(|now| now.attach.clone())
            })
        };
        for (index, a) in shown_walls.iter().enumerate() {
            for b in &shown_walls[index + 1..] {
                if (changed(a) || changed(b)) && group::overlaps(a, b) {
                    return Err(
                        "Walls cannot overlap: move it beside the other, or make one smaller.",
                    );
                }
            }
        }
        Ok(())
    }

    /// Whether every locked wall stands and is laid out as it was in `before`, so that a group carried, bent or settled
    /// moves none of them.
    pub(super) fn keeps_locked(&self, before: &Room) -> bool {
        self.walls.iter().filter(|added| added.locked).all(|added| {
            before
                .added(&added.id)
                .is_some_and(|was| was.pose == added.pose && was.layout == added.layout)
        })
    }

    /// Each added wall in a group of more than one, by identity: its group, named by its first wall, and its edges joined
    /// to the group's other walls.
    pub(super) fn joins(&self) -> BTreeMap<String, (String, Vec<Join>)> {
        let mut joins = BTreeMap::new();
        for added in &self.walls {
            let members = self.group(&added.id);
            if members.len() < 2 {
                continue;
            }
            let wall = shown(added);
            let joined = members[1..]
                .iter()
                .filter_map(|other| self.added(other))
                .map(shown)
                .filter_map(|other| {
                    group::joined(&wall, &other).map(|(edge, ..)| Join::new(edge, other.id))
                })
                .collect();
            joins.insert(added.id.clone(), (self.root(&added.id), joined));
        }
        joins
    }
}

impl Stage {
    /// Snap the added wall `wall` in `walls`' environment to another, as `attach` says, or, without, take it out of its
    /// group, standing at `pose` where given. Attached, the group it is in comes with it, unless `alone` takes it out of
    /// that first, as it does when the wall it is attached to is in the same group. Only walls someone added snap, both in
    /// this environment of this space, and none may then overlap another. Returns whether that changed anything, or why it
    /// was refused.
    pub(crate) fn attach_wall(
        &mut self,
        walls: &Walls<'_>,
        wall: &str,
        attach: Option<Attach>,
        alone: bool,
        pose: Option<Pose>,
    ) -> Result<bool, &'static str> {
        let added = |id: &str| {
            self.rooms
                .get(walls.environment)
                .is_some_and(|room| room.added(id).is_some())
        };
        // A wall snaps only to one `group::snaps_to` takes, as every client decides it, so that a wall that may not be a
        // target, such as one locked in place once walls can be locked, is refused here as well as never offered.
        let target = |id: &str| {
            self.shown_wall(walls, id)
                .is_some_and(|shown| added(id) && group::snaps_to(&shown))
        };
        let refusal = |id: &str| match self.shown_wall(walls, id) {
            Some(shown) if added(id) && shown.tilt() != 0.0 => {
                "A group's walls stand upright, so a tilted wall has none snapped to it: stand it upright first."
            }
            Some(_) => {
                "Only walls someone added snap together: the Tomori wall and an environment's walls stand where they are put."
            }
            None => "That wall is not here.",
        };
        if !added(wall) {
            return Err(refusal(wall));
        }
        if self
            .shown_wall(walls, wall)
            .is_some_and(|shown| shown.is_locked())
        {
            return Err(super::LOCKED);
        }
        if let Some(attach) = &attach {
            if !target(&attach.to) {
                return Err(refusal(&attach.to));
            }
            if attach.to == wall {
                return Err("A wall cannot be attached to itself.");
            }
            if !attach.known() {
                return Err("Tomori does not know that edge or alignment.");
            }
        }
        let pose = match pose {
            Some(pose) => {
                Some(valid(&pose).ok_or("A wall must stand within 30 m of the space's middle.")?)
            }
            None => None,
        };
        let room = self.room(walls.environment);
        let before = room.clone();
        match attach {
            None => {
                room.detach(wall);
                if let Some(pose) = pose {
                    room.added_mut(wall).expect("checked above").pose = pose;
                }
            }
            Some(attach) => {
                if alone || room.group(wall).contains(&attach.to) {
                    room.detach(wall);
                } else {
                    room.reroot(wall);
                }
                room.added_mut(wall).expect("checked above").attach = Some(attach);
            }
        }
        if let Err(error) = room.settle(&before) {
            *room = before;
            return Err(error);
        }
        if !room.keeps_locked(&before) {
            *room = before;
            return Err(super::GROUP_LOCKED);
        }
        Ok(room.walls != before.walls)
    }

    /// The walls of the group of the added wall `wall` in `walls`' environment, by identity, itself first: one alone is a
    /// group of one, and a wall not added, none.
    pub(crate) fn group_of(&self, walls: &Walls<'_>, wall: &str) -> Vec<String> {
        self.rooms
            .get(walls.environment)
            .filter(|room| room.added(wall).is_some())
            .map_or_else(Vec::new, |room| room.group(wall))
    }
}

#[cfg(test)]
mod tests;
