// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The simulation, on Rapier, which only a server runs: an environment's bodies, resting on its static colliders, taken
//! and let go by participants, falling, sliding and stacking once let go, damped and buoyed in its water, and asleep,
//! costing nothing, once they rest. Behind the `simulation` feature.
use std::collections::BTreeMap;

use glam::Vec3;
use rapier3d::parry::query::contact;
use rapier3d::parry::shape::Shape;
use rapier3d::prelude::{
    Collider, ColliderBuilder, ColliderHandle, PhysicsWorld, Pose3, QueryFilter, RigidBodyBuilder,
    RigidBodyHandle,
};

use crate::message::{
    Holding, Landed, LetGo, Returning, Shared, Snapshot, State, Take, Thrown, Update,
};
use crate::{
    BODY_LIMIT, Body, FADE, HELD_LIMIT, Held, Kind, Placement, Precision, Refused, SNAPSHOT_RATE,
    STATIC_TRIANGLES, Seen, Settings, Static, Water,
};

/// How far a body may lie within another where it would return to, in metres, for the place to be free: as one resting
/// on a neighbour that settled against it does, a millimetre or so, but not one lying where it was.
const OVERLAP: f32 = 0.005;
/// How sharply a body's velocity must change within a step for it to have struck something, in metres a second, and how
/// soon after one it may strike again, in milliseconds, so that a stone settling clacks once rather than rattling.
const STRUCK: f32 = 0.6;
const RESTRIKE: u64 = 150;
/// How far ahead of a body its contacts are foreseen, in metres: what a body let go at [`crate::RELEASE_SPEED`] travels in
/// a step at 60 Hz, and a little more.
const SOFT_CCD: f32 = 0.15;
/// How long everything that moves may move no more than [`STILL_MOVE`] metres, nor turn more than [`STILL_TURN`]
/// radians, before it is all put to sleep, in milliseconds: so that a body Rapier's own rule keeps awake, as a stone lying
/// where the ground bends, rocking at the solver's noise, rests all the same, and is said to, rather than being sent
/// forever.
const STILL_FOR: u64 = 2_000;
const STILL_MOVE: f32 = 0.003;
const STILL_TURN: f32 = 0.02;
/// How far a body let go inside another is lifted to clear it, at most, a centimetre at a time.
const CLEAR: f32 = 0.5;

/// Where a body is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Motion {
    /// Asleep, where it rests.
    Resting,
    /// Simulated, until it rests.
    Moving,
    /// In someone's hand: out of the simulation, touching nothing, until they let go.
    Held,
    /// On its way back to `to` since `began`, out of the simulation, touching nothing: where it was for [`FADE`],
    /// fading out, then at `to`, fading in, for [`FADE`] more.
    Returning { to: Placement, began: u64 },
}

/// One body of the world.
struct Entry {
    body: Body,
    handle: RigidBodyHandle,
    collider: ColliderHandle,
    epoch: u32,
    held: Option<Held>,
    motion: Motion,
    /// Where it began, as it settled there, which it returns to; or the home of a body alike that it took, as its own
    /// was taken.
    home: Placement,
    /// Its velocity a step ago, when it last struck something, and whether it was in water.
    velocity: Vec3,
    struck: u64,
    wet: bool,
    /// Since when it has rested in water that takes it back, on the world's clock.
    soaking: Option<u64>,
    /// Where it rests, as it was said, while it does.
    rested: crate::Encoded,
    /// Where it was when it last moved more than a little, and when, while it moves.
    still: Option<(u64, Placement)>,
}

/// An environment's bodies and what they rest on, simulated.
pub struct World {
    physics: PhysicsWorld,
    settings: Settings,
    entries: Vec<Entry>,
    numbers: BTreeMap<u32, usize>,
    waters: Vec<Water>,
    lowest: f32,
    /// How many steps it has taken, and the time it was given that was too little for another.
    steps: u64,
    remainder: f32,
    /// When it last said where what moves is, on its clock.
    said: Option<u64>,
    /// What changed that it has not yet said.
    pending: Update,
}

fn pose(placement: &Placement) -> Pose3 {
    Pose3::from_parts(placement.position, placement.rotation)
}

impl World {
    /// The world of `bodies` resting on `statics`, with `waters`, simulated by `settings`, each body asleep where it
    /// begins, snapped to how that is written at rest; or why it cannot be: more than [`BODY_LIMIT`] bodies, two with
    /// one number, a body [`Body::check`] refuses or whose hull has no volume, a static collider that is no shape, or
    /// more than [`STATIC_TRIANGLES`] triangles of them. See [`World::settle`] for bodies placed only roughly.
    pub fn new(
        settings: Settings,
        statics: Vec<Static>,
        waters: Vec<Water>,
        bodies: Vec<Body>,
    ) -> Result<Self, Refused> {
        if bodies.len() > BODY_LIMIT {
            return Err(Refused::Bodies);
        }
        if statics.iter().map(Static::triangles).sum::<usize>() > STATIC_TRIANGLES {
            return Err(Refused::Triangles);
        }
        let mut physics = PhysicsWorld::new();
        physics.gravity = settings.gravity;
        physics.integration_parameters.dt = settings.step;
        physics.integration_parameters.num_solver_iterations = settings.substeps.max(1);
        // Contacts between bodies as firm as Rapier makes those against the ground, so that a cairn of ten stones settles
        // no more than a few millimetres under its own weight rather than sinking and creeping.
        physics.integration_parameters.contact_softness =
            physics.integration_parameters.static_contact_softness;
        // Contacts against a mesh clustered, as Rapier's are by default, or each against its own triangle, as the
        // settings say.
        physics.integration_parameters.contact_clustering = settings.clustered;
        physics.integration_parameters.num_internal_pgs_iterations = settings.passes.max(1);
        let mut lowest = f32::INFINITY;
        for collider in &statics {
            if !collider.valid() {
                return Err(Refused::Static);
            }
            lowest = lowest.min(collider.lowest());
            let built = match collider {
                Static::Mesh {
                    vertices,
                    triangles,
                } => {
                    // Rapier refuses only a mesh of no triangles, which `valid` has, or one whose topology it was asked
                    // to check, which it is not.
                    let Ok(built) = ColliderBuilder::trimesh(vertices.clone(), triangles.clone())
                    else {
                        unreachable!("a mesh with triangles is a trimesh")
                    };
                    built
                }
                Static::Convex { points } => ColliderBuilder::convex_hull(points)
                    .filter(|built| built.build().volume() >= 1e-9)
                    .ok_or(Refused::Static)?,
                Static::Cuboid { placement, half } => {
                    ColliderBuilder::cuboid(half.x, half.y, half.z).position(pose(placement))
                }
            };
            physics.insert_collider(built.friction(0.9).restitution(0.05), None);
        }
        let mut world = Self {
            physics,
            settings,
            entries: Vec::new(),
            numbers: BTreeMap::new(),
            waters,
            lowest,
            steps: 0,
            remainder: 0.0,
            said: None,
            pending: Update::default(),
        };
        for body in bodies {
            world.insert(body)?;
        }
        Ok(world)
    }

    fn insert(&mut self, body: Body) -> Result<(), Refused> {
        body.check()?;
        if self.numbers.contains_key(&body.id) {
            return Err(Refused::Duplicate(body.id));
        }
        let collider = ColliderBuilder::convex_hull(&body.hull)
            .ok_or(Refused::Hull(body.id))?
            .density(body.density)
            .friction(body.friction)
            .restitution(body.restitution)
            .build();
        if collider.volume() < 1e-9 {
            return Err(Refused::Hull(body.id));
        }
        let home = body.home.snapped(Precision::Rest);
        let builder = match body.kind {
            Kind::Dynamic => RigidBodyBuilder::dynamic(),
            Kind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };
        let built = builder
            .pose(pose(&home))
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping)
            // Soft, predictive, rather than Rapier's hard CCD, which is more than a stone needs: far enough ahead to
            // catch one thrown at the fastest a hand lets go, 8 m/s, within a step.
            .soft_ccd_prediction(SOFT_CCD)
            .sleeping(true)
            .build();
        let (handle, collider) = self.physics.insert(built, collider);
        let activation = self.physics.bodies[handle].activation_mut();
        activation.normalized_linear_threshold = self.settings.sleep_speed;
        activation.angular_threshold = self.settings.sleep_turn;
        activation.time_until_sleep = self.settings.sleep_after;
        self.numbers.insert(body.id, self.entries.len());
        self.entries.push(Entry {
            body,
            handle,
            collider,
            epoch: 0,
            held: None,
            motion: Motion::Resting,
            home,
            velocity: Vec3::ZERO,
            struck: 0,
            wet: false,
            soaking: None,
            rested: home.encoded(Precision::Rest),
            still: None,
        });
        Ok(())
    }

    /// Let every body settle from where it was placed, for at most `seconds` of simulated time, until each rests, which
    /// is then where it begins and returns to, as for bodies placed roughly on uneven ground; says whether each did. What
    /// it changed is not said: a side settles its world before it shares it.
    pub fn settle(&mut self, seconds: f32) -> bool {
        for entry in &mut self.entries {
            if entry.body.kind == Kind::Dynamic {
                entry.motion = Motion::Moving;
                self.physics.bodies[entry.handle].wake_up(true);
            }
        }
        let steps = (seconds / self.settings.step).ceil() as u64;
        for _ in 0..steps {
            if !self.moving() {
                break;
            }
            self.advance();
        }
        for entry in &mut self.entries {
            entry.home = self.physics.bodies[entry.handle]
                .position()
                .into_placement();
            entry.soaking = None;
        }
        self.pending = Update::default();
        !self.moving()
    }

    /// Whether anything moves, so that its side steps it, and waits for intents otherwise.
    pub fn moving(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(entry.motion, Motion::Moving | Motion::Returning { .. })
                || entry.soaking.is_some()
        })
    }

    /// Its clock: how long it has simulated, in milliseconds.
    pub fn clock(&self) -> u64 {
        (self.steps as f64 * self.settings.step as f64 * 1000.0) as u64
    }

    /// Move on by `seconds`, at most a quarter of a second, in steps of its settings' step, and say what changed since it
    /// last said: where what moves is, at most [`SNAPSHOT_RATE`] times a second, what came to rest, changed hands or
    /// struck something.
    pub fn step(&mut self, seconds: f32) -> Update {
        self.remainder += if seconds.is_finite() {
            seconds.clamp(0.0, 0.25)
        } else {
            0.0
        };
        while self.remainder >= self.settings.step {
            self.remainder -= self.settings.step;
            self.advance();
        }
        let clock = self.clock();
        if self
            .said
            .is_none_or(|said| clock >= said + 1000 / SNAPSHOT_RATE as u64)
        {
            let moving: Vec<Snapshot> = self
                .entries
                .iter()
                .filter(|entry| entry.motion == Motion::Moving)
                .map(|entry| {
                    Snapshot::new(entry.body.id, &self.placement_of(entry), Precision::Moving)
                })
                .collect();
            if !moving.is_empty() {
                self.said = Some(clock);
                self.pending.moving = moving;
            }
        }
        std::mem::take(&mut self.pending)
    }

    fn placement_of(&self, entry: &Entry) -> Placement {
        self.physics.bodies[entry.handle]
            .position()
            .into_placement()
    }

    /// The water `point` is in, if any.
    fn water(&self, point: Vec3) -> Option<&Water> {
        self.waters.iter().find(|water| water.contains(point))
    }

    /// The water `body` placed at `at` is in, if any of it is, its middle within the water's outline, and how much of
    /// its height is below the water's surface, from 0 to 1.
    fn submerged(&self, body: &Body, at: &Placement) -> Option<(&Water, f32)> {
        let (lowest, highest) = body.hull.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(lowest, highest), point| {
                let y = at.transform_point(*point).y;
                (lowest.min(y), highest.max(y))
            },
        );
        let middle = glam::Vec2::new(at.position.x, at.position.z);
        let water = self
            .waters
            .iter()
            .find(|water| lowest < water.height && crate::body::within(&water.outline, middle))?;
        let under = ((water.height - lowest) / (highest - lowest).max(1e-4)).clamp(0.0, 1.0);
        Some((water, under))
    }

    /// One step.
    fn advance(&mut self) {
        let gravity = self.settings.gravity;
        for index in 0..self.entries.len() {
            let entry = &self.entries[index];
            if entry.motion == Motion::Moving {
                let at = self.placement_of(entry);
                // Buoyed by the water it displaces, its volume being its mass over its density, as much of it as
                // is below the water's surface, and damped more as it is: so that a body lighter than the water
                // floats, as a stick does, as deep as it is lighter, and comes to rest there.
                let (water, under) = match self.submerged(&entry.body, &at) {
                    Some((water, under)) => (Some(water.clone()), under),
                    None => (None, 0.0),
                };
                let (body, handle) = (&entry.body, entry.handle);
                let rigid = &mut self.physics.bodies[handle];
                rigid.reset_forces(false);
                match water {
                    Some(water) => {
                        let displaced = rigid.mass() / body.density * water.density;
                        rigid.add_force(-gravity * displaced * under, false);
                        let damped = |dry: f32, wet: f32| dry + (wet - dry) * under;
                        rigid.set_linear_damping(damped(body.linear_damping, water.linear_damping));
                        rigid.set_angular_damping(damped(
                            body.angular_damping,
                            water.angular_damping,
                        ));
                    }
                    None => {
                        rigid.set_linear_damping(body.linear_damping);
                        rigid.set_angular_damping(body.angular_damping);
                    }
                }
            }
        }
        self.physics.step();
        self.steps += 1;
        let clock = self.clock();
        for index in 0..self.entries.len() {
            self.after(index, clock);
        }
        // Everything that moves barely moving for a while, all of it is put to sleep together, and rests on the next
        // step, so that a cairn settling is never stopped part way.
        let mut moving = self
            .entries
            .iter()
            .filter(|entry| entry.motion != Motion::Resting && entry.motion != Motion::Held)
            .peekable();
        let quiet = moving.peek().is_some()
            && moving.all(|entry| {
                entry.motion == Motion::Moving
                    && entry
                        .still
                        .is_some_and(|(since, _)| clock >= since + STILL_FOR)
            });
        if quiet {
            for entry in self
                .entries
                .iter()
                .filter(|entry| entry.motion == Motion::Moving)
            {
                self.physics.bodies[entry.handle].sleep();
            }
        }
    }

    /// What became of body `index` in the step that ended at `clock`.
    fn after(&mut self, index: usize, clock: u64) {
        let entry = &self.entries[index];
        let rigid = &self.physics.bodies[entry.handle];
        let at = rigid.position().into_placement();
        let (velocity, sleeping) = (rigid.linvel(), rigid.is_sleeping());
        let id = entry.body.id;
        match entry.motion {
            Motion::Returning { to, began } if clock >= began + 2 * FADE => {
                // Clear of any body that has come to lie there since, but resting there on what it rests on.
                let home = self.clear(index, to, QueryFilter::exclude_fixed());
                let kind = entry.body.kind;
                let rigid = &mut self.physics.bodies[self.entries[index].handle];
                rigid.set_enabled(true);
                rigid.set_position(pose(&home), true);
                rigid.set_linvel(Vec3::ZERO, true);
                rigid.set_angvel(Vec3::ZERO, true);
                if kind == Kind::Kinematic {
                    // Only grabbable, it rests where it began, as it would where it was let go.
                    self.rest(index, home);
                } else {
                    self.entries[index].motion = Motion::Moving;
                }
            }
            Motion::Returning { to, began } if clock >= began + FADE => {
                // Faded out where it was, it is put where it returns to at once, still touching nothing, to fade in.
                self.physics.bodies[entry.handle].set_position(pose(&to), false);
            }
            Motion::Resting if !sleeping && entry.body.kind == Kind::Dynamic => {
                self.entries[index].motion = Motion::Moving
            }
            Motion::Resting => {
                let returns = self
                    .water(at.position)
                    .and_then(|water| water.returns_after);
                if let (Some(after), Some(since)) = (returns, self.entries[index].soaking)
                    && clock >= since + after
                {
                    self.send_back(index);
                }
            }
            Motion::Moving => {
                let water = self.water(at.position).is_some();
                let entry = &mut self.entries[index];
                // Struck something, or into water, as its velocity changed sharply, or it entered the water.
                let change = (velocity - entry.velocity).length();
                if (change >= STRUCK && clock >= entry.struck + RESTRIKE) || (water && !entry.wet) {
                    entry.struck = clock;
                    let speed = (change.max(velocity.length()) * 100.0).round() / 100.0;
                    self.pending.landed.push(Landed {
                        body: id,
                        speed,
                        water: water && !entry.wet,
                    });
                }
                entry.velocity = velocity;
                entry.wet = water;
                let lost = at.position.y < self.lowest - self.settings.lost_below
                    || (at.position - self.settings.home).with_y(0.0).length()
                        > self.settings.lost_beyond;
                // Where it was since it last moved more than a little.
                let still = entry.still.is_some_and(|(_, was)| {
                    was.position.distance(at.position) <= STILL_MOVE
                        && was.rotation.angle_between(at.rotation) <= STILL_TURN
                });
                if !still {
                    entry.still = Some((clock, at));
                }
                if lost {
                    self.send_back(index);
                } else if sleeping {
                    self.rest(index, at);
                }
            }
            _ => {}
        }
    }

    /// Body `index` resting at `at`, said once, as it is written at rest, which is what is kept and shared of it while it
    /// rests. Its body is not moved there, as moving a body wakes it: the two are within a twentieth of a millimetre
    /// and a few thousandths of a degree of each other, and it does not move while it sleeps.
    fn rest(&mut self, index: usize, at: Placement) {
        let encoded = at.encoded(Precision::Rest);
        let returns = self
            .water(at.position)
            .is_some_and(|water| water.returns_after.is_some());
        let clock = self.clock();
        let entry = &mut self.entries[index];
        entry.motion = Motion::Resting;
        entry.velocity = Vec3::ZERO;
        entry.still = None;
        entry.rested = encoded;
        entry.soaking = returns.then_some(clock);
        self.pending
            .rested
            .push(Snapshot::new(entry.body.id, &at, Precision::Rest));
    }

    /// `at`, lifted a centimetre at a time, at most [`CLEAR`], until body `index` placed there meets nothing `filter`
    /// takes: so that a body let go inside another, or the ground, is let go on it instead, one returned where another
    /// has come to lie is let go on that, but one returned where it rested rests there.
    fn clear(&self, index: usize, at: Placement, filter: QueryFilter) -> Placement {
        let entry = &self.entries[index];
        let shape = self.physics.colliders[entry.collider]
            .shared_shape()
            .clone();
        let filter = filter.exclude_rigid_body(entry.handle);
        let mut lifted = at;
        for _ in 0..(CLEAR * 100.0) as usize {
            if self
                .physics
                .intersect_shape(pose(&lifted), &*shape, filter)
                .next()
                .is_none()
            {
                return lifted;
            }
            lifted.position.y += 0.01;
        }
        lifted
    }

    fn index(&self, body: u32) -> Result<usize, Refused> {
        self.numbers
            .get(&body)
            .copied()
            .ok_or(Refused::Unknown(body))
    }

    /// Body `take.body` taken by participant `by`, last [`Seen`] as `seen`, if the side saw them, as their grip closes on
    /// it at the epoch they saw, if it is within their reach, nobody holds it, and they hold nothing in that hand and
    /// fewer than [`HELD_LIMIT`] bodies: out of the simulation, touching nothing, until they let go, and anything resting
    /// on it woken. Returns its epoch now.
    ///
    /// One they hold already, by the world's account, they take again, in the hand that takes it now, at the epoch they
    /// hold it at or the one their letting go of it would have made: their client let go of it, but its letting go never
    /// arrived or was refused, as one intent too many, so that they would otherwise never take it again until their client
    /// noticed. Its epoch is then the one after the epoch they asked at, as their client expects.
    pub fn take(&mut self, by: u32, take: &Take, seen: Option<&Seen>) -> Result<u32, Refused> {
        let index = self.index(take.body)?;
        let entry = &self.entries[index];
        let again = entry.held.is_some_and(|held| held.by == by)
            && (take.epoch == entry.epoch || take.epoch == entry.epoch.wrapping_add(1));
        if entry.held.is_some() && !again {
            return Err(Refused::Held(take.body));
        }
        if matches!(entry.motion, Motion::Returning { .. }) {
            return Err(Refused::Returning(take.body));
        }
        if entry.epoch != take.epoch && !again {
            return Err(Refused::Epoch(take.body));
        }
        let holding: Vec<&Held> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter_map(|(_, entry)| entry.held.as_ref())
            .filter(|held| held.by == by)
            .collect();
        if holding.len() >= HELD_LIMIT || holding.iter().any(|held| held.hand == take.hand) {
            return Err(Refused::Hands);
        }
        // Where it is: held, where the side last saw it held, or where it was taken.
        let at = match entry.held {
            Some(held) => seen
                .and_then(|seen| seen.holding(&held))
                .unwrap_or_else(|| self.placement_of(entry)),
            None => self.placement_of(entry),
        };
        take.reachable(at.position, entry.body.radius(), seen)?;
        if again {
            let entry = &mut self.entries[index];
            entry.epoch = take.epoch.wrapping_add(1);
            entry.held = Some(Held {
                by,
                hand: take.hand,
                offset: take.offset,
            });
            self.pending.holding.push(Holding {
                body: take.body,
                epoch: entry.epoch,
                held: entry.held,
            });
            return Ok(entry.epoch);
        }
        // Whatever rests on it or beside it wakes, so that a cairn whose stone is taken falls as it would.
        let reach = entry.body.radius();
        for other in 0..self.entries.len() {
            let near = self
                .placement_of(&self.entries[other])
                .position
                .distance(at.position)
                <= reach + self.entries[other].body.radius() + 0.05;
            if other != index && near && self.entries[other].motion == Motion::Resting {
                self.physics.bodies[self.entries[other].handle].wake_up(true);
                self.entries[other].motion = Motion::Moving;
            }
        }
        let entry = &mut self.entries[index];
        let rigid = &mut self.physics.bodies[entry.handle];
        rigid.set_enabled(false);
        entry.epoch = entry.epoch.wrapping_add(1);
        entry.held = Some(Held {
            by,
            hand: take.hand,
            offset: take.offset,
        });
        entry.motion = Motion::Held;
        entry.soaking = None;
        self.pending.holding.push(Holding {
            body: take.body,
            epoch: entry.epoch,
            held: entry.held,
        });
        Ok(entry.epoch)
    }

    /// Body `let_go.body` let go by its holder `by`, last [`Seen`] as `seen`, at the epoch their taking made, as
    /// [`LetGo::checked`] bounds it, and no faster than the body itself is let go ([`Body::thrown_at_most`]): back in
    /// the simulation where they let it go, lifted clear of anything it was let go inside, moving and spinning as it
    /// was; or, only grabbable, resting there. Returns its epoch now.
    pub fn let_go(&mut self, by: u32, let_go: &LetGo, seen: Option<&Seen>) -> Result<u32, Refused> {
        let index = self.index(let_go.body)?;
        let entry = &self.entries[index];
        let Some(held) = entry.held.filter(|held| held.by == by) else {
            return Err(Refused::Held(let_go.body));
        };
        if entry.epoch != let_go.epoch {
            return Err(Refused::Epoch(let_go.body));
        }
        let holding = seen
            .and_then(|seen| seen.holding(&held))
            .unwrap_or_else(|| self.placement_of(entry));
        let mut thrown = let_go.checked(seen, holding);
        thrown.velocity = thrown.velocity.clamp_length_max(entry.body.fastest);
        Ok(self.release(index, thrown))
    }

    /// Every body participant `by` holds let go where the side last saw their hand hold it, `seen`, if it did, or where
    /// each was taken, unmoving: as when they leave, or say nothing for a while. Returns those it let go.
    pub fn let_go_of(&mut self, by: u32, seen: Option<&Seen>) -> Vec<u32> {
        let held: Vec<usize> = (0..self.entries.len())
            .filter(|index| self.entries[*index].held.is_some_and(|held| held.by == by))
            .collect();
        held.into_iter()
            .map(|index| {
                let entry = &self.entries[index];
                let at = seen
                    .and_then(|seen| seen.holding(&entry.held.expect("held")))
                    .unwrap_or_else(|| self.placement_of(entry));
                self.release(
                    index,
                    Thrown {
                        at,
                        velocity: Vec3::ZERO,
                        spin: Vec3::ZERO,
                    },
                );
                self.entries[index].body.id
            })
            .collect()
    }

    /// Body `body`, which nobody holds, sent back to where it began, as a body lost or left in water is: fading out
    /// where it is over [`FADE`], put at once where it returns to, its home or the nearest free home of another, and
    /// fading in there over [`FADE`] more, nobody's to take until it is there; as an environment puts back what was left
    /// where it does not belong, or tidies. Its epoch is unchanged. Refused for a body someone holds; one on its way
    /// back already goes on its way.
    pub fn send_home(&mut self, body: u32) -> Result<(), Refused> {
        let index = self.index(body)?;
        match self.entries[index].motion {
            Motion::Held => Err(Refused::Held(body)),
            Motion::Returning { .. } => Ok(()),
            _ => {
                self.send_back(index);
                Ok(())
            }
        }
    }

    /// Body `index`, which nobody holds, taken back: out of the simulation, touching nothing, it fades out where it is
    /// for [`FADE`], is put, at once, where it returns to, [`World::free_home`], and fades in there for [`FADE`] more,
    /// before it is let go there, just clear of whatever has come to lie there since; said once, as it begins
    /// ([`Returning`]), so that every client fades it rather than drawing it on its way, and refused to anyone who
    /// would take it until it is there.
    fn send_back(&mut self, index: usize) {
        let id = self.entries[index].body.id;
        let from = self
            .placement(id)
            .expect("a body nobody holds has a placement");
        let to = self.free_home(index);
        let clock = self.clock();
        let entry = &mut self.entries[index];
        entry.motion = Motion::Returning { to, began: clock };
        entry.soaking = None;
        entry.still = None;
        entry.velocity = Vec3::ZERO;
        self.physics.bodies[entry.handle].set_enabled(false);
        self.pending.returning.push(Returning {
            body: id,
            from: from.encoded(Precision::Rest),
            to: to.encoded(Precision::Rest),
        });
    }

    /// Where body `index` returns to: its home, unless another body lies there, or is returning there; or else the
    /// nearest home of another body that is free, the body's home then that one, and that body's the one it left, if
    /// the two are alike, as two of the campsite's sticks are, so that neither then lies away from its home, or, if
    /// not, as two of the beach's stones are not, the place alone, its own turn, lifted clear of what it rests on; or,
    /// with nowhere free, its home all the same, lifted onto whatever lies there as it arrives.
    fn free_home(&mut self, index: usize) -> Placement {
        let own = self.entries[index].home;
        if !self.occupied(index, &own) {
            return own;
        }
        let mut others: Vec<usize> = (0..self.entries.len())
            .filter(|other| *other != index)
            .collect();
        let away = |other: &usize| self.entries[*other].home.position.distance(own.position);
        others.sort_by(|a, b| away(a).total_cmp(&away(b)));
        for other in others {
            let slot = self.entries[other].home;
            let alike = self.entries[other].body.hull == self.entries[index].body.hull;
            let at = if alike {
                slot
            } else {
                let turned = Placement::new(slot.position, own.rotation);
                self.clear(index, turned, QueryFilter::only_fixed())
            };
            if self.occupied(index, &at) {
                continue;
            }
            if alike {
                self.entries[other].home = own;
                self.entries[index].home = slot;
            }
            return at;
        }
        own
    }

    /// Whether body `index`, placed at `at`, would lie more than [`OVERLAP`] within another body in the simulation, or
    /// within where another is returning to.
    fn occupied(&self, index: usize, at: &Placement) -> bool {
        let entry = &self.entries[index];
        let shape = self.physics.colliders[entry.collider].shape();
        let placed = pose(at);
        let within = |position: &Pose3, other: &dyn Shape| {
            contact(&placed, shape, position, other, 0.0)
                .ok()
                .flatten()
                .is_some_and(|contact| contact.dist < -OVERLAP)
        };
        let away: Vec<ColliderHandle> = self
            .entries
            .iter()
            .filter(|other| matches!(other.motion, Motion::Held | Motion::Returning { .. }))
            .map(|other| other.collider)
            .collect();
        let present = |handle: ColliderHandle, _: &Collider| !away.contains(&handle);
        let filter = QueryFilter::exclude_fixed()
            .exclude_rigid_body(entry.handle)
            .predicate(&present);
        let lying = self
            .physics
            .intersect_shape(placed, shape, filter)
            .any(|(_, collider)| within(collider.position(), collider.shape()));
        lying
            || self.entries.iter().any(|other| match other.motion {
                Motion::Returning { to, .. } => {
                    within(&pose(&to), self.physics.colliders[other.collider].shape())
                }
                _ => false,
            })
    }

    /// Whether body `body` rests where it began, as it settled there, to within `near` metres.
    pub fn at_home(&self, body: u32, near: f32) -> bool {
        let Ok(index) = self.index(body) else {
            return false;
        };
        let entry = &self.entries[index];
        entry.motion == Motion::Resting
            && entry
                .rested
                .placement(Precision::Rest)
                .position
                .distance(entry.home.position)
                <= near
    }

    fn release(&mut self, index: usize, thrown: Thrown) -> u32 {
        let id = self.entries[index].body.id;
        self.physics.bodies[self.entries[index].handle].set_enabled(true);
        let at = self.clear(index, thrown.at, QueryFilter::default());
        let entry = &mut self.entries[index];
        entry.epoch = entry.epoch.wrapping_add(1);
        entry.held = None;
        entry.velocity = thrown.velocity;
        entry.wet = false;
        self.pending.holding.push(Holding {
            body: id,
            epoch: entry.epoch,
            held: None,
        });
        let epoch = entry.epoch;
        let rigid = &mut self.physics.bodies[entry.handle];
        rigid.set_position(pose(&at), true);
        if entry.body.kind == Kind::Kinematic {
            self.rest(index, at);
            return epoch;
        }
        rigid.set_linvel(thrown.velocity, true);
        rigid.set_angvel(thrown.spin, true);
        entry.motion = Motion::Moving;
        self.pending
            .moving
            .push(Snapshot::new(id, &at, Precision::Moving));
        epoch
    }

    /// Every body, as its side shares it for whoever joins.
    pub fn shared(&self) -> State {
        State {
            bodies: self
                .entries
                .iter()
                .map(|entry| Shared {
                    body: entry.body.id,
                    epoch: entry.epoch,
                    // One on its way back is shared where it returns to, as whoever joins sees it once it is there.
                    at: match entry.motion {
                        Motion::Resting => entry.rested,
                        Motion::Returning { to, .. } => to.encoded(Precision::Rest),
                        _ => self.placement_of(entry).encoded(Precision::Rest),
                    },
                    moving: entry.motion == Motion::Moving,
                    held: entry.held,
                })
                .collect(),
        }
    }

    /// Where body `body` is, unless someone holds it: where it was said to rest, while it rests.
    pub fn placement(&self, body: u32) -> Option<Placement> {
        let entry = &self.entries[self.index(body).ok()?];
        match entry.motion {
            Motion::Held => None,
            Motion::Resting => Some(entry.rested.placement(Precision::Rest)),
            _ => Some(self.placement_of(entry)),
        }
    }

    /// Who holds any body, by their numbers, in order.
    pub fn holders(&self) -> Vec<u32> {
        let mut holders: Vec<u32> = self
            .entries
            .iter()
            .filter_map(|entry| entry.held.map(|held| held.by))
            .collect();
        holders.sort_unstable();
        holders.dedup();
        holders
    }

    /// Body `body`'s epoch, if the world has it.
    pub fn epoch(&self, body: u32) -> Option<u32> {
        Some(self.entries[self.index(body).ok()?].epoch)
    }

    /// Who holds body `body`, if anyone does.
    pub fn held(&self, body: u32) -> Option<Held> {
        self.entries[self.index(body).ok()?].held
    }

    /// The numbers of its bodies, in the order they were given.
    pub fn bodies(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.iter().map(|entry| entry.body.id)
    }

    /// Where each body began, as it settled there.
    pub fn home(&self, body: u32) -> Option<Placement> {
        Some(self.entries[self.index(body).ok()?].home)
    }
}

/// A Rapier pose as a placement.
trait IntoPlacement {
    fn into_placement(self) -> Placement;
}

impl IntoPlacement for &Pose3 {
    fn into_placement(self) -> Placement {
        Placement::new(self.translation, self.rotation)
    }
}

#[cfg(test)]
mod tests;
