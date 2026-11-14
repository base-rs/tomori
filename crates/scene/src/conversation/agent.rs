// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent as the conversation shows it: the avatar it is drawn as, where it and its workers stand, and the hand
//! menu's ways to call it, send it home, pin it, walk with it and change how it moves.

use super::*;

impl Conversation {
    /// Show the agent as each avatar `avatars` names, by identifier, in a space whose backend names it, rather than as
    /// the swarm. See `design/avatars.md`.
    pub fn set_avatars(&mut self, avatars: Vec<(String, crate::avatar::Factory)>) {
        self.avatars = avatars;
    }

    /// Show as the swarm each avatar whose scenery `failures` says panicked in this build's renderer, which shares them,
    /// as `tomori-render`'s `Sceneries::avatar_failures` does. See `design/avatars.md`.
    pub fn set_avatar_failures(&mut self, failures: crate::avatar::Failures) {
        self.avatar_failures = failures;
    }

    /// The avatars this build registers, as [`Self::set_avatars`] gave them.
    #[cfg(any(test, feature = "testing"))]
    pub fn avatars(&self) -> &[(String, crate::avatar::Factory)] {
        &self.avatars
    }

    /// The avatar to show in the joined space, when its state names one this build registers; otherwise, as outside a
    /// space, none, the swarm. See `design/avatars.md`.
    pub fn avatar(&self) -> Option<crate::avatar::Choice<'_>> {
        let (space, _) = self.joined_space()?;
        let avatar = Self::named_avatar(space)?;
        self.avatars
            .iter()
            .find(|(registered, _)| registered == avatar)
            .map(|(_, factory)| crate::avatar::Choice {
                space: &space.id,
                avatar,
                factory,
                failures: &self.avatar_failures,
            })
    }

    /// The avatar `space`'s state names, if it names one a client reads.
    pub(super) fn named_avatar(space: &SpaceState) -> Option<&str> {
        space
            .avatar
            .as_deref()
            .filter(|avatar| tomori_protocol::valid_avatar(avatar))
    }

    /// The avatar `space`'s state names that this build lacks, when it shows the swarm instead. See
    /// `design/avatars.md`.
    pub(super) fn missing_avatar<'a>(&self, space: &'a SpaceState) -> Option<&'a str> {
        let avatar = Self::named_avatar(space)?;
        let lacks = avatar != crate::avatar::SWARM
            && !self
                .avatars
                .iter()
                .any(|(registered, _)| registered == avatar);
        lacks.then_some(avatar)
    }

    /// Where the agent and the workers stand now: in a space, the agent where it went or was called, facing whoever it
    /// turned to, or else at the home its state gives, and the workers about the workers anchor, or else the agent's
    /// home, gathered about the eyes at home, as they stay wherever the agent goes, each that is valid, by the placement
    /// registered for its environment where this build draws it at the space's version; and the carriage of that
    /// environment's own coordinates into the space's, where it draws it. Otherwise, as outside a space, the default
    /// layout. See `design/environments.md`.
    pub fn arrangement(&self) -> crate::anchors::Arrangement {
        let mut arrangement = crate::anchors::Arrangement::default();
        let Some((space, _)) = self.joined_space() else {
            return arrangement;
        };
        let anchor = |pose: Option<tomori_protocol::Pose>| {
            pose.as_ref().and_then(crate::anchors::Anchor::from_pose)
        };
        if let Some(home) = anchor(space.anchors.agent) {
            arrangement.home = home;
        }
        arrangement.workers = anchor(space.anchors.workers).unwrap_or(arrangement.home);
        // Where the ring drawn at home lies, as the home anchor says, whatever room or formation it gives.
        if let Some(home) = space.anchors.cluster(tomori_protocol::cluster::HOME) {
            arrangement.ring = home.ring;
        }
        // The room the home cluster may take, and how it stands, where the environment gives them, with the home anchor
        // where it says; room out of bounds is none.
        if let Some(home) = space
            .anchors
            .cluster(tomori_protocol::cluster::HOME)
            .filter(|home| {
                home.extent.is_some_and(|extent| extent.valid())
                    || home.formation == tomori_protocol::cluster::Formation::Ring
            })
            && let Some(workers) = anchor(Some(home.pose))
        {
            arrangement.workers = workers;
            arrangement.extent = home.extent.filter(|extent| extent.valid());
            if home.formation == tomori_protocol::cluster::Formation::Ring {
                arrangement.formation = tomori_protocol::cluster::Formation::Ring;
                arrangement.sweep = home
                    .sweep
                    .filter(|sweep| tomori_protocol::cluster::valid_sweep(*sweep));
            }
        }
        arrangement.agent = anchor(space.away).unwrap_or(arrangement.home);
        // Turned to whoever began speaking to it, about the vertical. See `design/environments.md`.
        if let Some(facing) = space
            .facing
            .map(glam::Vec3::from_array)
            .filter(|eyes| crate::anchors::within(*eyes, crate::anchors::REACH))
        {
            let at = arrangement.agent.position;
            let turned =
                tomori_protocol::Pose::at(at.to_array()).facing_towards([facing.x, at.y, facing.z]);
            arrangement.agent.orientation = glam::Quat::from_array(turned.orientation).normalize();
        }
        arrangement.via = space
            .via
            .map(glam::Vec3::from_array)
            .filter(|via| crate::anchors::within(*via, crate::anchors::REACH));
        arrangement.stream = space.stream;
        // Away, it draws in near whoever it stands for, or whoever it turned to.
        arrangement.partner = space
            .away
            .and(space.partner.or(space.facing))
            .map(glam::Vec3::from_array)
            .filter(|eyes| crate::anchors::within(*eyes, crate::anchors::REACH));
        // The space's other anchors, and where the server seats its workers, so that every client lays them out alike.
        arrangement.clusters = space
            .anchors
            .clusters
            .iter()
            .filter(|anchor| !anchor.is_home())
            .cloned()
            .collect();
        arrangement.layouts.clone_from(&space.clusters);
        // The environment drawn: one this build cannot draw at the space's version is the grid.
        let environment = self.environment_of(space);
        arrangement.placement = self
            .worker_placements
            .iter()
            .find(|(registered, _)| *registered == environment)
            .map(|(_, placement)| placement.clone());
        // Only the environment's own content is carried, and only where it is drawn: the grid stands as it is.
        if environment.id() == space.environment
            && environment != Environment::MINIMAL
            && let Some(carriage) = space
                .anchors
                .arrival
                .as_ref()
                .and_then(crate::anchors::Anchor::carriage)
        {
            arrangement.carriage = carriage;
        }
        arrangement
    }

    /// Whether this participant may send the agent home in the joined space: a participant, not a visitor, on a
    /// server that says how the agent moves. See `design/environments.md`.
    pub fn sends_agent_home(&self) -> bool {
        self.joined_space()
            .is_some_and(|(space, _)| space.movement.is_some() && self.role.participates())
    }

    /// How the joined space's agent moves, where the server says, and whether this participant may change it, as an
    /// owner may. See `design/environments.md`.
    pub fn agent_movement(&self) -> Option<(tomori_protocol::AgentMovement, bool)> {
        self.joined_space().and_then(|(space, _)| {
            space
                .movement
                .map(|movement| (movement, self.role == ConnectionRole::Owner))
        })
    }

    /// Send the joined space's agent home, for everyone; it goes home at once here, as everyone else is told. Nothing
    /// where this participant may not. See `design/environments.md`.
    pub fn send_agent_home(&mut self) {
        if !(self.sends_agent_home() && self.mode == Mode::Space) {
            return;
        }
        let space = self.space.as_mut().expect("in the space joined");
        space.away = None;
        space.facing = None;
        space.partner = None;
        self.send(ClientMessage::SendAgentHome);
    }

    /// Whether the hand menu offers **Call Tomori here**: to a participant, where the space lets the agent move, unless
    /// it is already pinned within [`CALLED_NEAR`] of them. See `design/hand-menu.md`.
    pub(super) fn offers_call(&self) -> bool {
        let Some((space, _)) = self.joined_space() else {
            return false;
        };
        let movable = space
            .movement
            .is_some_and(|movement| movement.mode != tomori_protocol::Movement::Home);
        let near = |at: &tomori_protocol::Pose| {
            self.head.is_some_and(|head| {
                glam::Vec3::from_array(head.position).distance(glam::Vec3::from_array(at.position))
                    <= CALLED_NEAR
            })
        };
        let here = space.pinned && space.away.as_ref().is_some_and(near);
        movable && self.role.participates() && !here
    }

    /// Bring the joined space's agent to this participant and pin it there, for everyone, as **Call Tomori here** does:
    /// where the space lets it move, on a server that says how it does. See `design/environments.md`.
    pub fn call_agent(&mut self) {
        if self.mode == Mode::Space
            && self.role.participates()
            && self
                .agent_movement()
                .is_some_and(|(movement, _)| movement.mode != tomori_protocol::Movement::Home)
        {
            self.send(ClientMessage::CallAgent);
        }
    }

    /// What the hand menu offers of the agent's pin, to a participant, where the space has it follow the speaker, on a
    /// server that takes pins: `Some(true)`, **Tomori: stay here**, while it is not pinned, and `Some(false)`, **Tomori:
    /// follow again**, while it is; none otherwise. See `design/hand-menu.md`.
    pub(super) fn offers_pin(&self) -> Option<bool> {
        let (space, _) = self.joined_space()?;
        let follows = space
            .movement
            .is_some_and(|movement| movement.mode == tomori_protocol::Movement::Follow);
        (space.pins && follows && self.role.participates()).then_some(!space.pinned)
    }

    /// Pin the joined space's agent where it stands, or, `pinned` false, let it follow the speaker again, for everyone,
    /// as **Tomori: stay here** and **Tomori: follow again** do: only where the hand menu offers it. See
    /// `design/environments.md`.
    pub fn pin_agent(&mut self, pinned: bool) {
        if self.mode == Mode::Space && self.offers_pin() == Some(pinned) {
            self.send(ClientMessage::PinAgent { pinned });
        }
    }

    /// What the hand menu offers of the agent walking with this participant, where the space lets it move, on a server
    /// that takes walks: `Some(true)`, **Tomori: walk with me**, while it walks with someone else or nobody, and
    /// `Some(false)`, **Tomori: stop walking with me**, while it walks with them; none otherwise. See
    /// `design/hand-menu.md`.
    pub(super) fn offers_walk(&self) -> Option<bool> {
        let (space, _) = self.joined_space()?;
        let moves = space
            .movement
            .is_some_and(|movement| movement.mode != tomori_protocol::Movement::Home);
        (space.walks && moves && self.role.participates()).then(|| {
            space
                .walking
                .is_none_or(|walker| Some(walker) != self.source)
        })
    }

    /// Have the joined space's agent walk with this participant, or, `walking` false, walk with them no more, as
    /// **Tomori: walk with me** and **Tomori: stop walking with me** do: only where the hand menu offers it. See
    /// `design/environments.md`.
    pub fn walk_agent(&mut self, walking: bool) {
        if self.mode == Mode::Space && self.offers_walk() == Some(walking) {
            self.send(ClientMessage::WalkAgent { walking });
        }
    }

    /// Change how the joined space's agent moves to `movement`, for everyone, as an owner. See
    /// `design/environments.md`.
    pub fn set_agent_movement(&mut self, movement: tomori_protocol::AgentMovement) {
        if self.mode == Mode::Space && self.agent_movement().is_some_and(|(_, owner)| owner) {
            self.send(ClientMessage::SetAgentMovement { movement });
        }
    }
}
