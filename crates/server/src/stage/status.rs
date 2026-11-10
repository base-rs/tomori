// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The space's status wall and its status panels: Tomori's notices, what the agent projects unasked and what the space
//! keeps on the Tomori wall, which belong to the space and go with it from one environment to the next. Where the status
//! wall is one of an environment's own, as the farmyard's notice board, its slots are shared with the environment's and
//! participants' panels there: status panels go into its free slots, those that do not fit wait, kept but not shown,
//! and take slots as they free, those a participant arranged first, then the newest. See `design/projection.md`.
use super::{Kept, Layout, Placed, Pose, Stage, Status, TOMORI_COLUMNS, Walls};
use tomori_protocol::{Grid, TOMORI_WALL};

/// Where a status panel the status wall gave up for an environment's own panel waits: no wall's identity, so that it
/// is seated again as a slot frees, in whichever environment the space is in.
const WAITING: &str = "waiting";

/// How many notices float side by side before a full status wall of an environment's own, each a slot's width further
/// to its right, before the next shares the first's place.
const BEFORE_END: usize = 6;

impl Layout {
    /// This layout, as the Tomori wall beneath the agent's home can stand it: one row of at most [`TOMORI_COLUMNS`],
    /// as many slots as it has, of its own size and shape, curved and tilted as it is, so that an edit the space made
    /// where the environment stood its status wall stands here as far as it can. A row of slots that small curves
    /// round less than half a circle at any radius a wall may have.
    fn beneath(self) -> Layout {
        Layout {
            grid: Grid::from((self.grid.slots().clamp(1, TOMORI_COLUMNS), 1)),
            scale: Layout::TOMORI.scale,
            aspect: None,
            ..self
        }
    }
}

impl Stage {
    /// How the Tomori wall is laid out in `walls`' environment: as the space changed it, over the layout its
    /// environment gives it; beneath the agent's home, within its own limits.
    pub(super) fn tomori_layout(&self, walls: &Walls<'_>) -> Layout {
        match walls.status {
            Status::At(layout) => self.tomori_layout.unwrap_or(layout),
            _ => self.tomori_layout.map_or(Layout::TOMORI, Layout::beneath),
        }
    }

    /// The panels that go on the wall `wall` in `walls`' environment, as status panels or not: the status panels for the
    /// Tomori wall, or, as `status` says, for an environment's own wall in the role; or else the environment's.
    pub(super) fn list_for(
        &mut self,
        walls: &Walls<'_>,
        wall: Option<&str>,
        status: bool,
    ) -> &mut Vec<Kept> {
        match wall {
            Some(TOMORI_WALL) => &mut self.tomori,
            Some(wall) if status && wall == walls.status_wall() => &mut self.tomori,
            _ => &mut self.room(walls.environment).panels,
        }
    }

    /// The slots of the wall `wall` in `walls`' environment that a panel is in: its own panels', and, on the status
    /// wall, the status panels'.
    pub(super) fn taken(&self, walls: &Walls<'_>, wall: &str) -> Vec<u32> {
        let room = self
            .rooms
            .get(walls.environment)
            .map(|room| room.panels.as_slice())
            .unwrap_or_default();
        let status: &[Kept] = if wall == walls.status_wall() {
            &self.tomori
        } else {
            &[]
        };
        room.iter()
            .chain(status)
            .filter_map(|kept| kept.slot_on(wall))
            .collect()
    }

    /// The most prominent free slot of the wall `wall` in `walls`' environment.
    pub(super) fn free_in(&self, walls: &Walls<'_>, wall: &str) -> Option<u32> {
        let taken = self.taken(walls, wall);
        (0..self.slots_of(walls, wall)).find(|slot| !taken.contains(slot))
    }

    /// Make room on the full status wall in `walls`' environment, an environment's own wall, for a panel, a status panel
    /// or not, and return the slot freed, or none. A status panel takes the slot of the oldest status panel nobody
    /// arranged, which leaves, as a full wall's oldest does; else of the oldest of the environment's own nobody arranged;
    /// and never one its environment published or a participant arranged. The environment's own panels come first: one
    /// takes the slot of the oldest status panel nobody arranged, which waits; else of the oldest of its own nobody
    /// arranged, which leaves; else of the oldest status panel, which waits; else of the oldest of its own, which leaves.
    /// A panel its environment published is never given up: the wall is not [`Stage::crowded`], which the caller sees
    /// to.
    pub(super) fn make_room(&mut self, walls: &Walls<'_>, status: bool) -> Option<u32> {
        let wall = walls.status_wall();
        let oldest = |list: &[Kept], arranged: Option<bool>| {
            list.iter()
                .enumerate()
                .filter(|(_, kept)| kept.published.is_none())
                .filter(|(_, kept)| arranged.is_none_or(|arranged| kept.arranged == arranged))
                .filter_map(|(index, kept)| {
                    kept.slot_on(&wall)
                        .map(|slot| (index, (kept.arranged, kept.placed), slot))
                })
                .min_by_key(|(_, order, _)| *order)
                .map(|(index, _, slot)| (index, slot))
        };
        let environment = walls.environment;
        let leaves = |stage: &mut Stage, index: usize| {
            stage.room(environment).panels.remove(index);
        };
        let waits = |stage: &mut Stage, index: usize| {
            let placed = stage.tick();
            let kept = &mut stage.tomori[index];
            kept.place = Placed::Wall {
                wall: WAITING.to_owned(),
                slot: 0,
            };
            kept.placed = placed;
        };
        let room = || {
            self.rooms
                .get(environment)
                .map(|room| room.panels.clone())
                .unwrap_or_default()
        };
        if status {
            if let Some((index, slot)) = oldest(&self.tomori, Some(false)) {
                self.tomori.remove(index);
                return Some(slot);
            }
            let (index, slot) = oldest(&room(), Some(false))?;
            leaves(self, index);
            return Some(slot);
        }
        if let Some((index, slot)) = oldest(&self.tomori, Some(false)) {
            waits(self, index);
            return Some(slot);
        }
        if let Some((index, slot)) = oldest(&room(), Some(false)) {
            leaves(self, index);
            return Some(slot);
        }
        if let Some((index, slot)) = oldest(&self.tomori, None) {
            waits(self, index);
            return Some(slot);
        }
        let (index, slot) = oldest(&room(), None)
            .expect("a full wall that is not crowded holds a panel to give up");
        leaves(self, index);
        Some(slot)
    }

    /// Make way, on the status wall in `walls`' environment, an environment's own, for one of the environment's own
    /// panels that names its slot `slot` on `wall`: a status panel there, with none of the wall's own, waits for another
    /// slot, since the environment's own panels come first. Returns whether one did.
    pub(super) fn give_way(&mut self, walls: &Walls<'_>, wall: &str, slot: u32) -> bool {
        if !walls.shared() || wall != walls.status_wall() || self.taken_by_own(walls, wall, slot) {
            return false;
        }
        let Some(index) = self
            .tomori
            .iter()
            .position(|kept| kept.slot_on(wall) == Some(slot))
        else {
            return false;
        };
        let placed = self.tick();
        let kept = &mut self.tomori[index];
        kept.place = Placed::Wall {
            wall: WAITING.to_owned(),
            slot: 0,
        };
        kept.placed = placed;
        true
    }

    /// Whether one of the environment's or participants' panels, not a status panel, is in the slot `slot` of `wall`
    /// in `walls`' environment.
    fn taken_by_own(&self, walls: &Walls<'_>, wall: &str, slot: u32) -> bool {
        self.rooms.get(walls.environment).is_some_and(|room| {
            room.panels
                .iter()
                .any(|kept| kept.slot_on(wall) == Some(slot))
        })
    }

    /// Where a notice floats that the full status wall `wall` in `walls`' environment, an environment's own, has no
    /// room for: just in front of its right end, facing as it faces, so that it is never lost; and, where one floats
    /// there already, a slot's width further to its right for each, so that notices never stand inside one another.
    pub(super) fn before_end(&self, walls: &Walls<'_>, wall: &str) -> Pose {
        let shown = self
            .shown_wall(walls, wall)
            .expect("the status wall is shown");
        let (width, _) = shown.size();
        let (slot, _) = shown.slot_size();
        let yaw = shown.pose.yaw();
        let [x, y, z] = shown.pose.position;
        let ahead = 0.25;
        let at = |step: usize| {
            let along = width / 2.0 - slot / 2.0 + slot * 1.1 * step as f32;
            let pose = Pose {
                position: [
                    x + along * yaw.cos() + ahead * yaw.sin(),
                    y,
                    z - along * yaw.sin() + ahead * yaw.cos(),
                ],
                orientation: shown.pose.orientation,
            };
            super::raised(super::valid(&pose).unwrap_or(shown.pose))
        };
        let floating: Vec<[f32; 3]> = self
            .rooms
            .get(walls.environment)
            .into_iter()
            .flat_map(|room| &room.panels)
            .filter_map(|kept| match &kept.place {
                Placed::Free { pose } => Some(pose.position),
                Placed::Wall { .. } => None,
            })
            .collect();
        let near = |pose: &Pose| {
            floating.iter().any(|position| {
                (0..3)
                    .map(|axis| (position[axis] - pose.position[axis]).powi(2))
                    .sum::<f32>()
                    < 0.05 * 0.05
            })
        };
        // As many steps as notices may float there, after which the first is shared.
        (0..BEFORE_END)
            .map(at)
            .find(|pose| !near(pose))
            .unwrap_or_else(|| at(0))
    }

    /// Whether the status panel `kept` is shown in `walls`' environment, in a slot of the status wall, rather than
    /// waiting for one.
    fn seated(&self, walls: &Walls<'_>, kept: &Kept, slots: u32) -> bool {
        kept.slot_on(&walls.status_wall())
            .is_some_and(|slot| slot < slots)
    }

    /// Seat the status panels waiting for a slot in `walls`' environment in the status wall's free slots, those a
    /// participant put where they are first, then the newest; those that do not fit wait, kept but not shown. Two in
    /// one slot, the older waits. Returns whether any moved.
    pub(crate) fn settle(&mut self, walls: &Walls<'_>) -> bool {
        let wall = walls.status_wall();
        let slots = self.slots_of(walls, &wall);
        let mut taken: Vec<u32> = self
            .rooms
            .get(walls.environment)
            .into_iter()
            .flat_map(|room| &room.panels)
            .filter_map(|kept| kept.slot_on(&wall))
            .collect();
        let mut order: Vec<usize> = (0..self.tomori.len()).collect();
        order.sort_by_key(|index| std::cmp::Reverse(self.tomori[*index].placed));
        let mut waiting = Vec::new();
        for index in order {
            let kept = &self.tomori[index];
            match kept.slot_on(&wall) {
                Some(slot) if self.seated(walls, kept, slots) && !taken.contains(&slot) => {
                    taken.push(slot);
                }
                _ => waiting.push(index),
            }
        }
        waiting.sort_by_key(|index| {
            let kept = &self.tomori[*index];
            (
                !(kept.pinned || kept.arranged),
                std::cmp::Reverse(kept.placed),
            )
        });
        let mut moved = false;
        for index in waiting {
            let Some(slot) = (0..slots).find(|slot| !taken.contains(slot)) else {
                // Those already on a wall that is not this environment's status wall wait where they are, so that they
                // are back in their slots where it is.
                if self.tomori[index].slot_on(&wall).is_some() {
                    self.tomori[index].place = Placed::Wall {
                        wall: WAITING.to_owned(),
                        slot: 0,
                    };
                    moved = true;
                }
                continue;
            };
            taken.push(slot);
            let placed = self.tick();
            let kept = &mut self.tomori[index];
            kept.place = Placed::Wall {
                wall: wall.clone(),
                slot,
            };
            kept.placed = placed;
            moved = true;
        }
        moved
    }

    /// The status panels waiting for a slot on the status wall in `walls`' environment, newest first, by identity and
    /// title, as the `panels` tool lists them.
    pub(crate) fn waiting(&self, walls: &Walls<'_>) -> Vec<(String, String)> {
        let slots = self.slots_of(walls, &walls.status_wall());
        let mut waiting: Vec<&Kept> = self
            .tomori
            .iter()
            .filter(|kept| !self.seated(walls, kept, slots))
            .collect();
        waiting.sort_by_key(|kept| std::cmp::Reverse(kept.placed));
        waiting
            .into_iter()
            .map(|kept| (kept.id.clone(), kept.content.title.clone()))
            .collect()
    }

    /// Whether the panel `panel` is one of the space's status panels, which go with it to the next environment's status
    /// wall.
    pub(crate) fn is_status(&self, panel: &str) -> bool {
        self.tomori.iter().any(|kept| kept.id == panel)
    }
}

#[cfg(test)]
mod tests;
