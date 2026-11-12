// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How participants, the agent and an environment add, change, lock and remove a space's walls, and what becomes of
//! the panels on them. See `design/projection.md`.
use tomori_protocol::{Pose, TOMORI_WALL, Wall, WallKind, wall as layout};

use super::{
    ADDED_WALLS, AddedWall, CONTENT_SCALE, DECLARED_WALLS, GROUP_LOCKED, Kept, LOCKED,
    LONGEST_RADIUS, Layout, Placed, SLOTS, Stage, Status, TOMORI_COLUMNS, WALL_SIZE, WallChange,
    Walls, valid,
};

impl Stage {
    /// Add a wall at `pose` in `walls`' environment, for its owners, laid out as `layout` says, or else flat, three
    /// columns of two rows. Returns its identity, or why it was refused.
    pub(crate) fn add_wall(
        &mut self,
        walls: &Walls<'_>,
        pose: &Pose,
        layout: Option<Layout>,
    ) -> Result<String, &'static str> {
        let pose = valid(pose).ok_or("A wall must stand within 30 m of the space's middle.")?;
        if self
            .rooms
            .get(walls.environment)
            .is_some_and(|room| room.walls.len() >= ADDED_WALLS)
        {
            return Err("This environment has as many added walls as it can take.");
        }
        let layout = layout.unwrap_or(Layout::FLAT);
        let wall = layout.apply(Wall::new("", WallKind::Added, pose));
        Self::fits(&wall, layout)?;
        self.has_room(walls, wall.slots() as usize)?;
        let id = format!("wall-{}", self.tick());
        self.room(walls.environment).walls.push(AddedWall {
            id: id.clone(),
            pose,
            layout: (layout != Layout::FLAT).then_some(layout),
            attach: None,
            locked: false,
        });
        Ok(id)
    }

    /// Whether `walls`' environment has room for walls of `more` slots than it has now, with the panels floating there,
    /// or why not.
    fn has_room(&self, walls: &Walls<'_>, more: usize) -> Result<(), &'static str> {
        let slots = self.slots_shown(walls) + more;
        if slots > SLOTS {
            return Err(
                "The walls here hold as many slots as they can together: make another wall smaller first.",
            );
        }
        if self.floating(walls.environment) > Self::floating_room(slots) {
            return Err(
                "Too many panels float here to give the walls more slots: put some on a wall or throw some away first.",
            );
        }
        Ok(())
    }

    /// Whether `wall`, laid out as `layout` says, is one a wall can be, or why not: its grid within bounds, its slots
    /// within [`layout::SCALES`] of a flat wall's, its curve, if it has one, of a radius from [`layout::LEAST_RADIUS`] to
    /// [`LONGEST_RADIUS`] about no more than half a circle, and no wider or taller than [`WALL_SIZE`].
    pub(crate) fn fits(wall: &Wall, layout: Layout) -> Result<(), &'static str> {
        if !layout.grid.valid() {
            return Err(
                "A wall has from 1 to 8 columns and from 1 to 4 rows of slots, and at most 24 slots.",
            );
        }
        if !(layout.scale.is_finite()
            && (layout::SCALES.0..=layout::SCALES.1).contains(&layout.scale))
        {
            return Err("A wall's slots are from half to five times a standard slot's size.");
        }
        if let Some(aspect) = layout.aspect
            && !(aspect.is_finite() && (layout::ASPECTS.0..=layout::ASPECTS.1).contains(&aspect))
        {
            return Err(
                "A wall's slots are from four times as tall as they are wide to four times as wide.",
            );
        }
        if let Some(radius) = layout.radius {
            if !(radius.is_finite() && (layout::LEAST_RADIUS..=LONGEST_RADIUS).contains(&radius)) {
                return Err("A wall is curved at a radius from 1 m to 50 m.");
            }
            if wall.radius().is_none() {
                return Err(
                    "Curved that tightly, its slots would go around more than half a circle: curve it less, or give it fewer columns.",
                );
            }
        }
        if let Some(tilt) = layout.tilt
            && !(tilt.is_finite() && (layout::TILTS.0..=layout::TILTS.1).contains(&tilt))
        {
            return Err("A wall tilts at most 45° from upright, either way.");
        }
        if let Some(content) = layout.content_scale
            && !(content.is_finite()
                && (wall.least_content_scale() - 1e-4..=layout::CONTENT_SCALES.1 + 1e-4)
                    .contains(&content))
        {
            return Err(CONTENT_SCALE);
        }
        let (width, height) = wall.size();
        if width > WALL_SIZE.0 + 1e-3 || height > WALL_SIZE.1 + 1e-3 {
            return Err(
                "A wall is at most 16 m wide and 9 m tall: give it fewer slots, or smaller ones.",
            );
        }
        Ok(())
    }

    /// Change the wall `wall` in `walls`' environment as `change` says: its grid, how large its slots are, whether it is
    /// curved, how far it tilts, and, for a wall an owner added or an environment's own, where it stands. A locked wall
    /// is refused whatever is asked. The Tomori wall stays one row of at most [`TOMORI_COLUMNS`], its slots its own size,
    /// where the environment puts it; an environment's own wall moved stands there for the space in that environment,
    /// over where the environment puts it. A panel keeps its column and row, or, in one the wall no longer has, goes into
    /// its nearest free slot, or else floats where it was: none is taken away. Returns whether that changed anything, or
    /// why it was refused.
    pub(crate) fn set_wall(
        &mut self,
        walls: &Walls<'_>,
        wall: &str,
        change: WallChange,
    ) -> Result<bool, &'static str> {
        let Some(before) = self.shown_wall(walls, wall) else {
            return Err("That wall is not here.");
        };
        if before.is_locked() {
            return Err(LOCKED);
        }
        let kind = before.kind;
        let old = Layout {
            grid: before.grid(),
            scale: before.scale(),
            aspect: before.aspect,
            radius: before.radius(),
            tilt: before.tilt,
            content_scale: before.content_scale,
        };
        // A standard slot's shape, said, is none.
        let aspect = change
            .aspect
            .unwrap_or(old.aspect)
            .filter(|aspect| aspect.is_nan() || (aspect - layout::STANDARD_ASPECT).abs() > 1e-4);
        let new = Layout {
            grid: change.grid.unwrap_or(old.grid),
            scale: change.scale.unwrap_or(old.scale),
            aspect,
            // Upright, said, is none, and a tilt is kept to a tenth of a degree.
            tilt: change
                .tilt
                .unwrap_or(old.tilt)
                .map(|tilt| (tilt * 10.0).round() / 10.0)
                .filter(|tilt| tilt.is_nan() || *tilt != 0.0),
            radius: change.radius.unwrap_or(old.radius),
            content_scale: None,
        };
        // As large as its slots give it, said, is none, and a content scale is kept to a hundredth, as its least is;
        // one kept, of slots made so small that it would draw their text below the least, is kept at the least.
        let least = Layout {
            content_scale: None,
            ..new
        }
        .apply(Wall::new(wall, kind, before.pose))
        .least_content_scale();
        let content = match change.content_scale {
            Some(asked) => asked.map(|scale| (scale * 100.0).round() / 100.0),
            None => old.content_scale.map(|kept| kept.max(least)),
        };
        let new = Layout {
            content_scale: content.filter(|scale| scale.is_nan() || *scale != 1.0),
            ..new
        };
        let pose = match (change.pose, kind) {
            (None, _) => before.pose,
            (Some(pose), WallKind::Added | WallKind::Environment) => {
                valid(&pose).ok_or("A wall must stand within 30 m of the space's middle.")?
            }
            (Some(_), _) => {
                return Err(
                    "The status wall stands where its environment puts it, so it cannot be moved.",
                );
            }
        };
        // Beneath the agent's home, it stays within its own limits; where its environment stands it, within a wall's.
        if kind == WallKind::Tomori && walls.status == Status::Beneath {
            if new.grid.rows != 1 || new.grid.columns > TOMORI_COLUMNS {
                return Err(
                    "The Tomori wall stays one row of at most 6 slots, so that it stays clear of the swarm and the workers.",
                );
            }
            if new.scale != old.scale || new.aspect != old.aspect {
                return Err(
                    "The Tomori wall's slots stay their size, so that it stays clear of the swarm and the workers.",
                );
            }
        }
        let after = new.apply(Wall::new(wall, kind, pose));
        Self::fits(&after, new)?;
        if new == old && pose == before.pose {
            return Ok(false);
        }
        let more = after.slots() as usize;
        let fewer = before.slots() as usize;
        if more > fewer {
            self.has_room(walls, more - fewer)?;
        }
        // Kept as the Tomori wall's for the space, an environment wall's for the space in its environment, or the added
        // wall's own.
        match kind {
            WallKind::Tomori => {
                let default = match walls.status {
                    Status::At(layout) => layout,
                    _ => Layout::TOMORI,
                };
                self.tomori_layout = (new != default).then_some(new);
            }
            WallKind::Environment => {
                let declared =
                    Self::declared(walls, wall).expect("an environment's wall shown is declared");
                let room = self.room(walls.environment);
                if new == declared.layout() {
                    room.layouts.remove(wall);
                } else {
                    room.layouts.insert(wall.to_owned(), new);
                }
                if pose == declared.pose {
                    room.moved.remove(wall);
                } else {
                    room.moved.insert(wall.to_owned(), pose);
                }
            }
            _ => {
                let room = self.room(walls.environment);
                let unchanged = room.clone();
                let members = room.group(wall);
                if members.len() > 1 && new.tilt.is_some() {
                    return Err(
                        "A group's walls stand upright, so that it reads as one board: take this wall out of its group to tilt it.",
                    );
                }
                let added = room
                    .walls
                    .iter_mut()
                    .find(|added| added.id == wall)
                    .expect("an added wall shown is kept");
                added.layout = (new != Layout::FLAT).then_some(new);
                if members.len() > 1 {
                    // A group's walls are curved alike, so that one bent bends them all; carried, it carries them all.
                    if change.radius.is_some() {
                        for member in &members[1..] {
                            let added = room
                                .walls
                                .iter_mut()
                                .find(|added| added.id == *member)
                                .expect("a wall of the group is kept");
                            let mut layout = added.layout.unwrap_or(Layout::FLAT);
                            layout.radius = new.radius;
                            let bent = layout.apply(Wall::new(member.clone(), kind, added.pose));
                            if let Err(error) = Self::fits(&bent, layout) {
                                *room = unchanged;
                                return Err(error);
                            }
                            added.layout = (layout != Layout::FLAT).then_some(layout);
                        }
                    }
                    if pose != before.pose {
                        room.carry(wall, pose);
                    }
                    if let Err(error) = room.settle(&unchanged) {
                        *room = unchanged;
                        return Err(error);
                    }
                    if !room.keeps_locked(&unchanged) {
                        *room = unchanged;
                        return Err(GROUP_LOCKED);
                    }
                } else {
                    added.pose = pose;
                }
            }
        }
        self.rearrange(walls, &before, &after);
        Ok(true)
    }

    /// Put the panels on the wall `before` as it has become, `after`: each in the same column and row, or, in one it no
    /// longer has, in its nearest free slot, those a participant put where they are first and then the newest, or else
    /// floating where its slot was, facing as it faced.
    fn rearrange(&mut self, walls: &Walls<'_>, before: &Wall, after: &Wall) {
        // On an environment's own wall in the status role, the status panels there keep their column and row as its own
        // panels do, and go into its nearest free slot or float as they do; floating, they are the environment's.
        let status: Vec<String> = if walls.shared() && before.id == walls.status_wall() {
            let (on, rest): (Vec<Kept>, Vec<Kept>) = std::mem::take(&mut self.tomori)
                .into_iter()
                .partition(|kept| kept.slot_on(&before.id).is_some());
            self.tomori = rest;
            let ids = on.iter().map(|kept| kept.id.clone()).collect();
            self.room(walls.environment).panels.extend(on);
            ids
        } else {
            Vec::new()
        };
        self.rearranged(walls, before, after);
        if !status.is_empty() {
            let room = self.room(walls.environment);
            let (back, rest): (Vec<Kept>, Vec<Kept>) = std::mem::take(&mut room.panels)
                .into_iter()
                .partition(|kept| status.contains(&kept.id) && kept.slot_on(&before.id).is_some());
            room.panels = rest;
            self.tomori.extend(back);
        }
        // Grown again, as when put back as its environment has it, its new slots take the status panels waiting.
        if walls.shared() && before.id == walls.status_wall() {
            self.settle(walls);
        }
    }

    fn rearranged(&mut self, walls: &Walls<'_>, before: &Wall, after: &Wall) {
        let (old, new) = (before.grid(), after.grid());
        let environment = walls.environment;
        let list = self.list(environment, Some(&before.id));
        let mut kept: Vec<u32> = Vec::new();
        let mut displaced: Vec<(usize, (u32, u32))> = Vec::new();
        for (index, panel) in list.iter_mut().enumerate() {
            let Some(slot) = panel.slot_on(&before.id) else {
                continue;
            };
            let (column, row) = old.cell(slot).unwrap_or((u32::MAX, u32::MAX));
            match new.slot(column, row) {
                Some(slot) => {
                    panel.place = Placed::Wall {
                        wall: before.id.clone(),
                        slot,
                    };
                    kept.push(slot);
                }
                None => displaced.push((index, (column.min(old.columns), row.min(old.rows)))),
            }
        }
        // Those a participant put where they are first, then the newest.
        displaced.sort_by_key(|(index, _)| {
            let panel = &list[*index];
            (
                !(panel.pinned || panel.arranged),
                std::cmp::Reverse(panel.placed),
            )
        });
        let mut floating = Vec::new();
        for (index, (column, row)) in displaced {
            let nearest = (0..new.slots())
                .filter(|slot| !kept.contains(slot))
                .min_by_key(|slot| {
                    let (at, down) = new.cell(*slot).expect("a slot of the grid");
                    let across = i64::from(at) - i64::from(column);
                    let down = i64::from(down) - i64::from(row);
                    (across * across + down * down, *slot)
                });
            let panel = &mut list[index];
            match nearest {
                Some(slot) => {
                    panel.place = Placed::Wall {
                        wall: before.id.clone(),
                        slot,
                    };
                    kept.push(slot);
                }
                None => {
                    let slot = old.slot(column, row).unwrap_or_default();
                    floating.push((panel.id.clone(), before.slot_pose(slot)));
                }
            }
        }
        // Floating, a panel belongs to the environment, as one a participant leaves floating does; none is taken away.
        for (id, pose) in floating {
            let mut panel = self.take(environment, &id).expect("a panel on the wall");
            panel.place = Placed::Free {
                pose: pose.and_then(|pose| valid(&pose)).unwrap_or(before.pose),
            };
            panel.size = None;
            panel.arranged = false;
            panel.placed = self.tick();
            self.list(environment, None).push(panel);
        }
    }

    /// Remove the added wall `wall` from `walls`' environment, leaving each panel on it floating where its slot was,
    /// facing as it faced. Returns why it was refused.
    pub(crate) fn remove_wall(
        &mut self,
        walls: &Walls<'_>,
        wall: &str,
    ) -> Result<(), &'static str> {
        let added = self
            .rooms
            .get(walls.environment)
            .is_some_and(|room| room.walls.iter().any(|added| added.id == wall));
        let shown = self.shown_wall(walls, wall).filter(|_| added);
        let Some(shown) = shown else {
            return Err(if wall == TOMORI_WALL || wall.starts_with("environment-") {
                "Only a wall an owner added can be removed."
            } else {
                "That wall is not here."
            });
        };
        if shown.is_locked() {
            return Err(LOCKED);
        }
        let tick = self.tick();
        let room = self.room(walls.environment);
        // Taken out of its group first, the rest stay together where they still touch.
        room.detach(wall);
        room.walls.retain(|added| added.id != wall);
        // Its context goes with it, so that a wall added later with its identity begins following.
        room.contexts.remove(wall);
        for panel in &mut room.panels {
            if let Some(slot) = panel.slot_on(wall) {
                panel.place = Placed::Free {
                    pose: shown
                        .slot_pose(slot)
                        .and_then(|pose| valid(&pose))
                        .unwrap_or(shown.pose),
                };
                panel.size = None;
                panel.arranged = false;
                panel.placed = tick;
            }
        }
        Ok(())
    }

    /// Put each panel kept on a wall in `walls`' environment in a slot the wall no longer has, as when its environment
    /// lays out its own wall anew, with fewer slots, into the wall's first free slot, or else floating beside the wall, to
    /// its right, so that none is kept but hidden. Returns whether it moved any.
    pub(crate) fn fit_slots(&mut self, walls: &Walls<'_>) -> bool {
        let environment = walls.environment;
        let mut moved = false;
        for wall in self.walls(walls) {
            let slots = wall.slots();
            let list = self.list(environment, Some(&wall.id));
            let beyond: Vec<String> = list
                .iter()
                .filter(|kept| kept.slot_on(&wall.id).is_some_and(|slot| slot >= slots))
                .map(|kept| kept.id.clone())
                .collect();
            let mut floating = 0;
            for id in beyond {
                moved = true;
                if let Some(slot) = self.free_in(walls, &wall.id) {
                    let list = self.list(environment, Some(&wall.id));
                    let kept = list
                        .iter_mut()
                        .find(|kept| kept.id == id)
                        .expect("a panel on the wall");
                    kept.place = Placed::Wall {
                        wall: wall.id.clone(),
                        slot,
                    };
                    continue;
                }
                // Floating beside the wall, to its right, a slot's width further for each, facing as it faces.
                let (width, _) = wall.size();
                let (slot_width, _) = wall.slot_size();
                let along = width / 2.0 + slot_width * (0.6 + 1.1 * floating as f32);
                let yaw = wall.pose.yaw();
                let [x, y, z] = wall.pose.position;
                let pose = Pose {
                    position: [
                        x + along * yaw.cos() + 0.1 * yaw.sin(),
                        y,
                        z - along * yaw.sin() + 0.1 * yaw.cos(),
                    ],
                    orientation: wall.pose.orientation,
                };
                floating += 1;
                let mut kept = self.take(environment, &id).expect("a panel on the wall");
                kept.place = Placed::Free {
                    pose: valid(&pose).unwrap_or(wall.pose),
                };
                kept.size = None;
                kept.arranged = false;
                kept.placed = self.tick();
                self.list(environment, None).push(kept);
            }
        }
        moved
    }

    /// The environment's own wall `wall` as `walls`' environment declares it, if it is one.
    fn declared<'a>(walls: &Walls<'a>, wall: &str) -> Option<&'a crate::EnvironmentWall> {
        let index: usize = wall
            .strip_prefix("environment-")
            .and_then(|index| index.parse().ok())?;
        walls
            .declared
            .iter()
            .take(DECLARED_WALLS)
            .nth(index)
            .filter(|_| format!("environment-{index}") == wall)
    }

    /// Lock the wall `wall` in `walls`' environment, or unlock it, for the space: the Tomori wall's in every
    /// environment, an environment's own wall's or an added wall's in this one. With `reset`, an environment's own wall
    /// is put back first as the environment has it, where it stands, its grid, slots and curve, its panels keeping their
    /// column and row or else going into its nearest free slots or floating where they were, and it is locked, as the
    /// environment has it. Returns whether that changed anything, or why it was refused. Who may ask is the caller's to
    /// check: owners. See `design/projection.md`.
    pub(crate) fn lock_wall(
        &mut self,
        walls: &Walls<'_>,
        wall: &str,
        locked: bool,
        reset: bool,
    ) -> Result<bool, &'static str> {
        let Some(before) = self.shown_wall(walls, wall) else {
            return Err("That wall is not here.");
        };
        if reset {
            let Some(declared) = Self::declared(walls, wall).copied() else {
                return Err(
                    "Only an environment's own wall is put back as the environment has it.",
                );
            };
            if !locked {
                return Err("A wall put back as the environment has it is locked, as it has it.");
            }
            let after = declared
                .layout()
                .apply(Wall::new(wall, WallKind::Environment, declared.pose))
                .with_name(declared.name().map(str::to_owned))
                .with_locked(true);
            if after == before {
                return Ok(false);
            }
            if after.slots() > before.slots() {
                self.has_room(walls, (after.slots() - before.slots()) as usize)?;
            }
            let room = self.room(walls.environment);
            room.layouts.remove(wall);
            room.moved.remove(wall);
            room.unlocked.remove(wall);
            self.rearrange(walls, &before, &after);
            return Ok(true);
        }
        if before.is_locked() == locked {
            return Ok(false);
        }
        match before.kind {
            WallKind::Tomori => self.tomori_locked = locked,
            WallKind::Environment => {
                let unlocked = &mut self.room(walls.environment).unlocked;
                if locked {
                    unlocked.remove(wall);
                } else {
                    unlocked.insert(wall.to_owned());
                }
            }
            _ => {
                let room = self.room(walls.environment);
                let added = room
                    .walls
                    .iter_mut()
                    .find(|added| added.id == wall)
                    .expect("an added wall shown is kept");
                added.locked = locked;
            }
        }
        Ok(true)
    }
}
