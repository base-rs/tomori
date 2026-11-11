// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::anchors::Anchor;
use crate::interaction::{Mode, Visual};
use glam::Vec3;

pub const AGENT_COUNT: usize = 128;
pub const CENTER: Vec3 = Vec3::new(0.0, 1.4, -2.3);

/// How fast its agents fly at most while the swarm stays where it is, in metres a second.
pub const FLIGHT: f32 = 0.85;

/// How long its agents take to set off, one after another, as it flies between the lobby and a space, in seconds, so
/// that they stream across rather than move as one. See [`Swarm::fly_to`].
pub const STAGGER: f32 = 0.6;

/// How quickly the fastest its agents may fly comes back down to [`FLIGHT`] once they have arrived, in metres a second
/// each second: no faster than they can slow themselves, so that none is ever braked.
const UNWIND: f32 = 3.0;

/// How near where it stands every agent is once a flight has arrived, in metres: about as far as its shapes reach.
const ARRIVED: f32 = 1.5;

/// How long a flight lasts at most, in seconds, should its agents never all arrive, as while the trigger draws them.
const LONGEST: f32 = 10.0;

#[derive(Clone, Copy)]
pub struct Agent {
    pub position: Vec3,
    pub velocity: Vec3,
}

/// A flight of its agents from where the swarm stood, `from`, to where it stands, as this client's participant joins or
/// leaves a space: each sets off `elapsed` seconds in, after those before it, and they stream there at up to
/// [`crate::glide::STREAM`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct Flight {
    from: Anchor,
    elapsed: f32,
}

impl Flight {
    /// Whether the agent `index` has set off.
    fn set_off(&self, index: usize) -> bool {
        self.elapsed >= STAGGER * index as f32 / AGENT_COUNT as f32
    }
}

pub struct Swarm {
    pub agents: Vec<Agent>,
    pub visual: Visual,
    next: Vec<Agent>,
    time: f32,
    /// Where its middle stands and which way its eye faces: [`Anchor::AGENT`] unless the joined space's environment
    /// puts the agent elsewhere. See `design/environments.md`.
    anchor: Anchor,
    /// How far its middle moved since the last step, so that its agents keep up with it as it glides or streams.
    moved: f32,
    /// How large it is drawn, as a share of its full size: smaller near whoever it stands for.
    size: f32,
    /// Its agents' flight to where it stands, while they make it. See [`Self::fly_to`].
    flight: Option<Flight>,
    /// How fast its agents may fly at most, coming back down to [`FLIGHT`] after a flight.
    cap: f32,
}

impl Default for Swarm {
    fn default() -> Self {
        // A deterministic spherical distribution: no RNG or assets required.
        let agents: Vec<_> = (0..AGENT_COUNT)
            .map(|i| {
                let y = 1.0 - 2.0 * (i as f32 + 0.5) / AGENT_COUNT as f32;
                let angle = i as f32 * 2.399_963_1;
                let r = (1.0 - y * y).sqrt();
                let offset = Vec3::new(r * angle.cos(), y * 0.6, r * angle.sin());
                Agent {
                    position: CENTER + offset * 0.85,
                    velocity: Vec3::new(-offset.z, 0.05, offset.x) * 0.35,
                }
            })
            .collect();
        Self {
            next: agents.clone(),
            agents,
            visual: Visual::default(),
            time: 0.0,
            anchor: Anchor::AGENT,
            moved: 0.0,
            size: 1.0,
            flight: None,
            cap: FLIGHT,
        }
    }
}

impl Swarm {
    /// Smooth the audio envelope, but never smooth away a discrete turn state.
    pub fn set_visual(&mut self, mut visual: Visual, dt: f32) {
        visual.time = self.visual.time + dt.clamp(0.0, 0.1);
        let rate = if visual.level > self.visual.level {
            24.0
        } else {
            9.0
        };
        visual.level = self.visual.level
            + (visual.level - self.visual.level) * (1.0 - (-rate * dt.clamp(0.0, 0.1)).exp());
        self.visual = visual;
    }

    /// Draw in towards `size`, a share of its full size, over about a second, as a flock gathers to listen or spreads
    /// out again.
    pub fn draw_in(&mut self, size: f32, dt: f32) {
        let size = size.clamp(0.1, 1.0);
        self.size += (size - self.size) * (1.0 - (-3.0 * dt.clamp(0.0, 0.1)).exp());
    }

    /// How large it is drawn, as a share of its full size.
    pub fn size(&self) -> f32 {
        self.size
    }

    /// Stand at `anchor` from now on: the agents glide there, as they move between shapes, as when the agent goes to
    /// someone or home, keeping up with its middle as it moves.
    pub fn set_anchor(&mut self, anchor: Anchor) {
        self.moved += anchor.position.distance(self.anchor.position);
        self.anchor = anchor;
    }

    /// How fast its agents fly at most over `dt` seconds: [`FLIGHT`], or a quarter faster than its middle moved, so that
    /// they keep up with it as it streams; or, in a flight, [`crate::glide::STREAM`], coming back down by [`UNWIND`]
    /// once they have arrived.
    fn fastest(&mut self, dt: f32) -> f32 {
        let pace = if dt > 0.0 { self.moved / dt } else { 0.0 };
        self.moved = 0.0;
        self.cap = if self.flight.is_some() {
            crate::glide::STREAM
        } else {
            (self.cap - UNWIND * dt).max(FLIGHT)
        };
        self.cap.max(pace * 1.25)
    }

    /// Appear at `anchor` from now on: the agents are carried there at once, in the shape they have, turned with it,
    /// rather than gliding, as when a space opens with the agent at its home. See `design/environments.md`. In a flight,
    /// they fly on there instead, so that a flight never ends in a jump.
    pub fn appear_at(&mut self, anchor: Anchor) {
        if anchor == self.anchor {
            return;
        }
        if self.flight.is_some() {
            self.anchor = anchor;
            return;
        }
        self.carry(self.anchor, anchor);
        self.anchor = anchor;
    }

    /// Fly to `anchor` from where its agents are, as this client's participant joins or leaves a space: they set off
    /// one after another over [`STAGGER`] and stream there at up to [`crate::glide::STREAM`], as they do when the agent
    /// streams, keeping up with `anchor` as it moves on from there; turned back on their way, as by leaving the space
    /// they were flying into, they all turn back at once, never jumping. See `design/arrival.md`.
    pub fn fly_to(&mut self, anchor: Anchor) {
        if anchor == self.anchor && self.flight.is_none() {
            return;
        }
        self.flight = Some(Flight {
            from: self.anchor,
            elapsed: if self.flight.is_some() { STAGGER } else { 0.0 },
        });
        self.anchor = anchor;
    }

    /// Whether its agents are flying to where it stands. See [`Self::fly_to`].
    pub fn flying(&self) -> bool {
        self.flight.is_some()
    }

    /// Carried as what stood at `from` is carried to `to`, in the shape it has, turned with it, as when the
    /// participant's room is moved in the scene under them: its agents and where it stands, or, in a flight, where its
    /// agents fly from, though not where they fly to.
    pub fn carry(&mut self, from: Anchor, to: Anchor) {
        let turn = to.orientation * from.orientation.inverse();
        let carried = |position: Vec3| to.position + turn * (position - from.position);
        for agent in self.agents.iter_mut().chain(self.next.iter_mut()) {
            agent.position = carried(agent.position);
            agent.velocity = turn * agent.velocity;
        }
        let anchor = match &mut self.flight {
            Some(flight) => &mut flight.from,
            None => &mut self.anchor,
        };
        *anchor = Anchor {
            position: carried(anchor.position),
            orientation: (turn * anchor.orientation).normalize(),
        };
    }

    /// Where its middle stands and which way its eye faces.
    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    /// The point `local`, relative to its middle as it faces home by default, where it stands now, as its rings are
    /// drawn about it.
    pub fn at(&self, local: Vec3) -> Vec3 {
        self.anchor.place(local)
    }

    pub fn step(&mut self, dt: f32, attractor: Option<Vec3>) {
        // Bound stalls (sleep/resume) rather than launching agents across the room.
        let dt = dt.clamp(0.0, 1.0 / 30.0);
        self.time += dt;
        // Drawn by the trigger, its agents go to it as ever, and a flight ends.
        if attractor.is_some() {
            self.flight = None;
        }
        if let Some(flight) = &mut self.flight {
            flight.elapsed += dt;
        }
        let fastest = self.fastest(dt);
        let size = self.size;
        let flight = self.flight;
        // Where the agent `index` stands about: where the swarm stands, or, until it sets off, where it flies from.
        let anchor = self.anchor;
        let about = move |index: usize| match flight {
            Some(flight) if !flight.set_off(index) => flight.from,
            _ => anchor,
        };
        let wander = Vec3::new((self.time * 0.3).sin() * 0.35 * size, 0.0, 0.0);
        self.step_agents(dt, attractor, fastest, &about, wander);
        // Once every agent has set off and arrived, the flight is over.
        if let Some(flight) = self.flight {
            let arrived = flight.elapsed >= STAGGER
                && self
                    .agents
                    .iter()
                    .all(|agent| agent.position.distance(self.anchor.position) <= ARRIVED);
            if arrived || flight.elapsed >= LONGEST {
                self.flight = None;
            }
        }
    }

    /// Move its agents on by `dt` seconds at up to `fastest`, each about where `about` says, to `attractor` where it is
    /// given, or, as a flock, wandering by `wander` about its own place.
    fn step_agents(
        &mut self,
        dt: f32,
        attractor: Option<Vec3>,
        fastest: f32,
        about: &dyn Fn(usize) -> Anchor,
        wander: Vec3,
    ) {
        let size = self.size;
        let target = |index: usize| attractor.unwrap_or_else(|| about(index).place(wander));
        if self.visual.mode != Mode::Neutral && attractor.is_none() {
            // A damped spring keeps state changes gentle and world-anchored.
            // Shape and motion, not color alone, distinguish the main states.
            for (i, agent) in self.agents.iter_mut().enumerate() {
                let t = i as f32 / AGENT_COUNT as f32;
                let (radius, height, depth, speed) = match self.visual.mode {
                    Mode::Working => (0.52, 0.36, 0.42, 0.8),
                    Mode::Preparing | Mode::Transcribing => (0.65, 0.42, 0.32, 0.45),
                    Mode::Ready => (0.94, 0.50, 0.20, 0.06),
                    Mode::Waiting => (0.85, 0.52, 0.17, 0.03),
                    Mode::Listening => (0.98, 0.44, 0.14, 0.08),
                    Mode::Speaking => (0.80, 0.44, 0.27, 0.16),
                    _ => (0.70, 0.40, 0.20, 0.025),
                };
                let angle = t * std::f32::consts::TAU + self.visual.time * speed;
                let ring = 0.75 + 0.25 * (i % 3) as f32 / 2.0;
                let ripple = if self.visual.mode == Mode::Speaking {
                    self.visual.amplitude() * (angle * 3.0 - self.visual.time * 5.0).sin() * 0.12
                } else {
                    0.0
                };
                let goal = about(i).place(
                    Vec3::new(
                        angle.cos() * radius * ring,
                        angle.sin() * height * ring + ripple,
                        (angle * 3.0 + i as f32).sin() * depth,
                    ) * size,
                );
                let force = (goal - agent.position) * 5.0 - agent.velocity * 3.8;
                agent.velocity =
                    (agent.velocity + force.clamp_length_max(3.0) * dt).clamp_length_max(fastest);
                agent.position += agent.velocity * dt;
            }
            return;
        }
        for (i, agent) in self.agents.iter().enumerate() {
            let target = target(i);
            let mut separation = Vec3::ZERO;
            let mut alignment = Vec3::ZERO;
            let mut cohesion = Vec3::ZERO;
            let mut neighbors = 0.0;
            for (j, other) in self.agents.iter().enumerate() {
                if i == j {
                    continue;
                }
                let delta = agent.position - other.position;
                let d2 = delta.length_squared();
                if d2 < (0.22 * size).powi(2) {
                    separation += delta / d2.max(0.0025);
                }
                if d2 < (0.65 * size).powi(2) {
                    alignment += other.velocity;
                    cohesion += other.position;
                    neighbors += 1.0;
                }
            }
            let attraction = if attractor.is_some() { 1.5 } else { 0.7 };
            let mut force =
                separation * 0.055 + (target - agent.position) * attraction - agent.velocity * 0.45;
            if neighbors > 0.0 {
                force += (alignment / neighbors - agent.velocity) * 0.55;
                force += (cohesion / neighbors - agent.position) * 0.22;
            }
            let offset = agent.position - target;
            force += Vec3::new(-offset.z, (self.time + i as f32).sin() * 0.12, offset.x) * 0.3;
            // Keep agents above the floor even when a controller is lowered.
            force.y += (0.35 - agent.position.y).max(0.0) * 5.0;
            let velocity =
                (agent.velocity + force.clamp_length_max(3.0) * dt).clamp_length_max(fastest);
            self.next[i] = Agent {
                position: agent.position + velocity * dt,
                velocity,
            };
        }
        std::mem::swap(&mut self.agents, &mut self.next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_with_no_neighbours_still_moves() {
        let mut swarm = Swarm::default();
        for (i, agent) in swarm.agents.iter_mut().enumerate() {
            agent.position = Vec3::new(i as f32 * 10.0, 0.0, 0.0);
        }
        let before = swarm.agents[0].position;
        swarm.step(0.1, None);
        assert!(swarm.agents[0].position.is_finite());
        assert_ne!(swarm.agents[0].position, before);
    }

    #[test]
    fn it_draws_in_near_whoever_it_stands_for_and_spreads_out_again() {
        // How far across its agents reach, about the vertical.
        let across = |swarm: &Swarm| {
            swarm
                .agents
                .iter()
                .map(|agent| (agent.position.x - CENTER.x).hypot(agent.position.z - CENTER.z))
                .fold(0.0f32, f32::max)
                * 2.0
        };
        for mode in [Mode::Ready, Mode::Speaking, Mode::Neutral] {
            let mut swarm = Swarm::default();
            let settle = |swarm: &mut Swarm, size: f32| {
                for _ in 0..720 {
                    swarm.set_visual(
                        Visual {
                            mode,
                            ..Default::default()
                        },
                        1.0 / 72.0,
                    );
                    swarm.draw_in(size, 1.0 / 72.0);
                    swarm.step(1.0 / 72.0, None);
                }
            };
            settle(&mut swarm, 1.0);
            let full = across(&swarm);
            assert_eq!(swarm.size(), 1.0);
            settle(&mut swarm, crate::avatar::DRAWN);
            let drawn = across(&swarm);
            assert!(
                (swarm.size() - crate::avatar::DRAWN).abs() < 1e-3,
                "{mode:?}"
            );
            assert!(drawn < full * 0.7, "{mode:?}: {drawn} of {full}");
            settle(&mut swarm, 1.0);
            assert!(across(&swarm) > full * 0.85, "{mode:?}");
        }
        // Never smaller than a tenth, however it is asked.
        let mut swarm = Swarm::default();
        for _ in 0..720 {
            swarm.draw_in(0.0, 1.0 / 72.0);
        }
        assert!((swarm.size() - 0.1).abs() < 1e-3);
    }

    #[test]
    fn conversational_shapes_stay_finite_and_settle_without_teleporting() {
        let mut swarm = Swarm::default();
        for mode in [
            Mode::Working,
            Mode::Speaking,
            Mode::Ready,
            Mode::Waiting,
            Mode::Listening,
            Mode::Unavailable,
        ] {
            for _ in 0..360 {
                let previous = swarm.agents[0].position;
                swarm.set_visual(
                    Visual {
                        mode,
                        level: 0.8,
                        ..Default::default()
                    },
                    1.0 / 72.0,
                );
                swarm.step(1.0 / 72.0, None);
                assert!(swarm.agents[0].position.distance(previous) <= 0.012);
                for agent in &swarm.agents {
                    assert!(agent.position.is_finite());
                    assert!(agent.position.distance(CENTER) < 1.5);
                }
            }
        }
    }

    #[test]
    fn swarm_is_finite_bounded_and_deterministic() {
        let mut a = Swarm::default();
        let mut b = Swarm::default();
        for _ in 0..1500 {
            a.step(1.0 / 72.0, None);
            b.step(1.0 / 72.0, None);
        }
        assert_eq!(a.agents.len(), AGENT_COUNT);
        for (a, b) in a.agents.iter().zip(&b.agents) {
            assert!(a.position.is_finite());
            assert!(a.position.distance(CENTER) < 3.0);
            assert!(a.velocity.length() <= 0.851);
            assert_eq!(a.position, b.position);
        }
    }

    #[test]
    fn trigger_attractor_moves_the_swarm() {
        let mut swarm = Swarm::default();
        let target = CENTER + Vec3::X * 1.5;
        for _ in 0..1000 {
            swarm.step(1.0 / 72.0, Some(target));
        }
        let mean = swarm.agents.iter().map(|a| a.position).sum::<Vec3>() / AGENT_COUNT as f32;
        assert!(
            mean.distance(target) < 0.6,
            "centroid {mean}, target {target}"
        );
    }

    #[test]
    fn the_swarm_glides_to_its_anchor_turned_with_it_and_back() {
        let mut swarm = Swarm::default();
        assert_eq!(swarm.anchor(), Anchor::AGENT);
        assert_eq!(swarm.at(Vec3::X), CENTER + Vec3::X);
        let anchor = Anchor {
            position: Vec3::new(3.0, 1.8, -4.0),
            orientation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        swarm.set_anchor(anchor);
        // Turned a quarter, its middle's +X is −Z.
        assert!(swarm.at(Vec3::X).distance(anchor.position - Vec3::Z) < 1e-5);
        let mean = |swarm: &Swarm| {
            swarm.agents.iter().map(|a| a.position).sum::<Vec3>() / AGENT_COUNT as f32
        };
        for mode in [Mode::Neutral, Mode::Ready] {
            swarm.set_visual(
                Visual {
                    mode,
                    ..Default::default()
                },
                0.0,
            );
            for _ in 0..1500 {
                let previous = swarm.agents[0].position;
                swarm.step(1.0 / 72.0, None);
                // It glides, never jumps.
                assert!(swarm.agents[0].position.distance(previous) <= 0.012);
            }
            assert!(mean(&swarm).distance(anchor.position) < 0.6, "{mode:?}");
        }
        // Its eye faces along the anchor's +Z: in its ring, the agents spread along −Z and Y, not X.
        let spread = swarm
            .agents
            .iter()
            .map(|a| (a.position - anchor.position).abs())
            .fold(Vec3::ZERO, Vec3::max);
        assert!(spread.z > spread.x, "{spread}");
        swarm.set_anchor(Anchor::AGENT);
        for _ in 0..1500 {
            swarm.step(1.0 / 72.0, None);
        }
        assert!(mean(&swarm).distance(CENTER) < 0.6);
    }

    #[test]
    fn the_swarm_flies_to_its_anchor_its_agents_setting_off_one_after_another_and_arrives() {
        for mode in [Mode::Neutral, Mode::Ready] {
            let mut swarm = Swarm::default();
            swarm.set_visual(
                Visual {
                    mode,
                    ..Default::default()
                },
                0.0,
            );
            for _ in 0..200 {
                swarm.step(1.0 / 72.0, None);
            }
            let start: Vec<Vec3> = swarm.agents.iter().map(|agent| agent.position).collect();
            let there = Anchor {
                position: Vec3::new(-3.0, 1.5, -1.0),
                orientation: glam::Quat::from_rotation_y(1.0),
            };
            swarm.fly_to(there);
            assert!(swarm.flying());
            assert_eq!(swarm.anchor(), there);
            // A third of the way through setting off, the first agents are on their way, the last not yet.
            for _ in 0..(STAGGER / 3.0 * 72.0) as usize {
                swarm.step(1.0 / 72.0, None);
            }
            let towards = (there.position - CENTER).normalize();
            let gone = |index: usize| (swarm.agents[index].position - start[index]).dot(towards);
            let first: f32 = (0..16).map(gone).sum();
            let last: f32 = (AGENT_COUNT - 16..AGENT_COUNT).map(gone).sum();
            assert!(first > last + 0.1, "{mode:?}: {first} {last}");
            // They stream there, never faster than the swarm streams, and arrive within seconds.
            let mut frames = 0;
            while swarm.flying() {
                let before: Vec<Vec3> = swarm.agents.iter().map(|agent| agent.position).collect();
                swarm.step(1.0 / 72.0, None);
                for (agent, before) in swarm.agents.iter().zip(&before) {
                    assert!(agent.position.distance(*before) <= crate::glide::STREAM / 72.0 + 1e-5);
                }
                frames += 1;
                assert!(frames < 72 * 4, "{mode:?}");
            }
            assert!(
                swarm
                    .agents
                    .iter()
                    .all(|agent| agent.position.distance(there.position) <= ARRIVED)
            );
            // Once arrived, they are soon as slow as ever.
            for _ in 0..144 {
                swarm.step(1.0 / 72.0, None);
            }
            assert!(
                swarm
                    .agents
                    .iter()
                    .all(|agent| agent.velocity.length() <= FLIGHT + 1e-4)
            );
            // Flying to where it is already does nothing.
            swarm.fly_to(there);
            assert!(!swarm.flying());
        }
    }

    #[test]
    fn a_flight_is_carried_with_the_room_flies_on_where_it_would_appear_and_ends_when_drawn() {
        let mut swarm = Swarm::default();
        let there = Anchor {
            position: Vec3::new(3.0, 1.5, -1.0),
            orientation: glam::Quat::IDENTITY,
        };
        swarm.fly_to(there);
        for _ in 0..10 {
            swarm.step(1.0 / 72.0, None);
        }
        // Carried with the room, a quarter turn and a metre along: its agents are carried, and where they fly from, but
        // not where they fly to.
        let before: Vec<Vec3> = swarm.agents.iter().map(|agent| agent.position).collect();
        let from = Anchor::HOME;
        let to = Anchor {
            position: Vec3::X,
            orientation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        swarm.carry(from, to);
        for (agent, before) in swarm.agents.iter().zip(&before) {
            assert!(agent.position.distance(to.place(*before)) < 1e-4);
        }
        assert_eq!(swarm.anchor(), there);
        let flight = swarm.flight.expect("flying");
        assert!(flight.from.position.distance(to.place(CENTER)) < 1e-4);
        // Told to appear elsewhere as it flies, it flies on there instead, without a jump.
        let elsewhere = Anchor {
            position: Vec3::new(-2.0, 1.5, -1.0),
            orientation: glam::Quat::IDENTITY,
        };
        let before: Vec<Vec3> = swarm.agents.iter().map(|agent| agent.position).collect();
        swarm.appear_at(elsewhere);
        assert!(swarm.flying());
        assert_eq!(swarm.anchor(), elsewhere);
        assert!(
            swarm
                .agents
                .iter()
                .zip(&before)
                .all(|(agent, before)| agent.position == *before)
        );
        // Drawn by the trigger, its flight is over.
        swarm.step(1.0 / 72.0, Some(Vec3::new(0.0, 1.2, -1.0)));
        assert!(!swarm.flying());
        // Carried while it stands still, where it stands is carried with it.
        swarm.carry(from, to);
        assert!(
            swarm
                .anchor()
                .position
                .distance(to.place(elsewhere.position))
                < 1e-4
        );
    }

    #[test]
    fn zero_delta_and_resume_stalls_are_safe() {
        let mut swarm = Swarm::default();
        let start = swarm.agents[0].position;
        swarm.step(0.0, None);
        assert_eq!(start, swarm.agents[0].position);
        swarm.step(100.0, None);
        assert!(start.distance(swarm.agents[0].position) < 0.03);
    }

    #[test]
    fn the_swarm_appears_at_its_home_at_once_in_the_shape_it_has() {
        let mut swarm = Swarm::default();
        for _ in 0..200 {
            swarm.step(1.0 / 72.0, None);
        }
        let before: Vec<Vec3> = swarm
            .agents
            .iter()
            .map(|agent| agent.position - CENTER)
            .collect();
        let home = Anchor {
            position: Vec3::new(-1.0, 1.6, -5.0),
            orientation: glam::Quat::IDENTITY,
        };
        swarm.appear_at(home);
        assert_eq!(swarm.anchor(), home);
        for (agent, offset) in swarm.agents.iter().zip(&before) {
            assert!(agent.position.distance(home.position + *offset) < 1e-4);
        }
        // Already there, nothing moves.
        let there: Vec<Vec3> = swarm.agents.iter().map(|agent| agent.position).collect();
        swarm.appear_at(home);
        assert!(
            swarm
                .agents
                .iter()
                .zip(&there)
                .all(|(agent, at)| agent.position == *at)
        );
    }
}
