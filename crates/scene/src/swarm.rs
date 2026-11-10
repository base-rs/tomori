// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::anchors::Anchor;
use crate::interaction::{Mode, Visual};
use glam::Vec3;

pub const AGENT_COUNT: usize = 128;
pub const CENTER: Vec3 = Vec3::new(0.0, 1.4, -2.3);

/// How fast its agents fly at most while the swarm stays where it is, in metres a second.
pub const FLIGHT: f32 = 0.85;

#[derive(Clone, Copy)]
pub struct Agent {
    pub position: Vec3,
    pub velocity: Vec3,
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

    /// Stand at `anchor` from now on: the agents glide there, as they move between shapes, as when the agent goes to
    /// someone or home, keeping up with its middle as it moves.
    pub fn set_anchor(&mut self, anchor: Anchor) {
        self.moved += anchor.position.distance(self.anchor.position);
        self.anchor = anchor;
    }

    /// How fast its agents fly at most over `dt` seconds: [`FLIGHT`], or a quarter faster than its middle moved, so that
    /// they keep up with it as it streams.
    fn fastest(&mut self, dt: f32) -> f32 {
        let pace = if dt > 0.0 { self.moved / dt } else { 0.0 };
        self.moved = 0.0;
        FLIGHT.max(pace * 1.25)
    }

    /// Appear at `anchor` from now on: the agents are carried there at once, in the shape they have, turned with it,
    /// rather than gliding, as when a space opens with the agent at its home. See `design/environments.md`.
    pub fn appear_at(&mut self, anchor: Anchor) {
        if anchor == self.anchor {
            return;
        }
        let back = self.anchor.orientation.inverse();
        let turn = anchor.orientation * back;
        for agent in self.agents.iter_mut().chain(self.next.iter_mut()) {
            agent.position = anchor.position + turn * (agent.position - self.anchor.position);
            agent.velocity = turn * agent.velocity;
        }
        self.anchor = anchor;
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
        let fastest = self.fastest(dt);
        let target = attractor
            .unwrap_or_else(|| self.at(Vec3::new((self.time * 0.3).sin() * 0.35, 0.0, 0.0)));
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
                let goal = self.anchor.place(Vec3::new(
                    angle.cos() * radius * ring,
                    angle.sin() * height * ring + ripple,
                    (angle * 3.0 + i as f32).sin() * depth,
                ));
                let force = (goal - agent.position) * 5.0 - agent.velocity * 3.8;
                agent.velocity =
                    (agent.velocity + force.clamp_length_max(3.0) * dt).clamp_length_max(fastest);
                agent.position += agent.velocity * dt;
            }
            return;
        }
        for (i, agent) in self.agents.iter().enumerate() {
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
                if d2 < 0.22 * 0.22 {
                    separation += delta / d2.max(0.0025);
                }
                if d2 < 0.65 * 0.65 {
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
