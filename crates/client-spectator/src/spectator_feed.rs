// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spectator's live feed: connects to a space the same way the headset does
//! (`Conversation` + `Connection`), tracks participant presence relayed by the
//! server, and synthesizes the followed participant's point of view into a
//! [`Frame`] for the shared spectator renderer.
//!
//! The feed is polled once per rendered frame, on the render thread. It is
//! non-blocking: the connection's `poll` drains whatever events are waiting,
//! and the presence hook (set via [`Connection::watch_presence`]) writes each
//! relayed presence into a shared map that the feed reads after the poll.
use crate::{
    capture,
    connection::Connection,
    controller::Pose,
    conversation::{Conversation, Mode},
    delegation::{Scene, Workers},
    environment::Environment,
    identity::{Identity, Overrides, Store},
    interaction, speaker,
    spectator::Frame,
    swarm::{CENTER, Swarm},
};
use glam::{Mat4, Quat, Vec3};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tomori_protocol::{Hand, ServerMessage};
use tomori_render::scenery::Sceneries;
use tomori_scene::avatar::Coordinator;

/// A participant's latest presence report, as relayed by the server.
#[derive(Clone)]
struct ParticipantPresence {
    head: tomori_protocol::Pose,
    left: Option<tomori_protocol::Pose>,
    right: Option<tomori_protocol::Pose>,
    hands: Option<[Option<Hand>; 2]>,
}

/// Animate the coordinator from the space's observed activity and the agent
/// speech the spectator itself plays, as the headset does: the swarm, or the
/// avatar the space's backend names where the spectator's sceneries register
/// it. Presence does not currently carry the headset's swarm or attractor, so
/// this is a local simulation rather than an exact copy of its particles.
struct SwarmAnimation {
    signals: interaction::Signals,
    coordinator: Coordinator,
    /// Where the agent stands, as the space's environment puts it.
    anchor: tomori_scene::anchors::Anchor,
    /// The followed participant's head, which an avatar may face.
    head: Option<Vec3>,
    /// Whether the agent appears at its anchor this frame, rather than gliding there.
    appear: bool,
}

impl SwarmAnimation {
    fn new() -> Self {
        Self {
            signals: interaction::Signals::default(),
            coordinator: Coordinator::default(),
            anchor: tomori_scene::anchors::Anchor::AGENT,
            head: None,
            appear: false,
        }
    }

    /// The swarm to draw: the coordinator's, or none while another avatar shows.
    fn swarm(&self) -> &Swarm {
        self.coordinator.swarm()
    }

    fn update(
        &mut self,
        session: interaction::SessionActivity,
        speech: interaction::SpeechActivity,
        dt: f32,
    ) {
        let visual = self.signals.update(
            interaction::Inputs {
                session,
                speech,
                ..Default::default()
            },
            dt,
        );
        self.coordinator.update(
            &tomori_scene::avatar::Input::new(visual, self.anchor, dt)
                .appearing(std::mem::take(&mut self.appear))
                .speaking(speech.playing)
                .head(self.head),
        );
    }
}

/// The options a spectator is started with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpectatorOptions {
    /// A server address to connect to straight away.
    pub server: Option<String>,
    /// A space id to join, as printed by `tomori discover`.
    pub space: Option<String>,
    /// A participant name to follow, or the first available.
    pub follow: Option<String>,
    /// A token for the server, for a server that refuses visitors. See `design/security.md`.
    pub token: Option<String>,
}

/// The spectator's connection to a space. Tracks participant presence and
/// synthesizes the followed participant's view into render frames.
pub struct Feed {
    conversation: Conversation,
    connection: Connection,
    /// The latest presence for each participant source number, written by the
    /// presence hook during `Connection::poll` and read after.
    presence: Arc<Mutex<BTreeMap<u32, ParticipantPresence>>>,
    /// The participant name to follow, or `None` for the first available.
    follow: Option<String>,
    /// The source number of the participant currently being followed.
    following: Option<u32>,
    /// The name of the participant currently being followed, for the caption.
    following_name: Option<String>,
    /// Worker simulation state, derived from the space's workers.
    workers: Workers,
    scene: Scene,
    swarm: SwarmAnimation,
    /// The agent's home as the swarm last stood at it, so that it appears at a new one, as the headset's does.
    agent_home: tomori_scene::anchors::Anchor,
    /// The ring drawn on the floor at home, which stays there wherever the agent goes, as the headset draws it.
    home_ring: tomori_scene::anchors::HomeRing,
    sequence: u64,
    /// The scenery shown outside a space, or in one from a server that names no environment, from `TOMORI_ENVIRONMENT`.
    environment: Environment,
    /// The sceneries the spectator draws, as [`Self::with_sceneries`] gave them: the grid alone until then.
    sceneries: Sceneries,
    last_poll: Instant,
    last_caption: Option<String>,
}

impl Feed {
    /// Connect to the server in `options.server`, join `options.space`, and
    /// follow `options.follow`, hearing the space through the desktop's speaker.
    pub fn new(options: SpectatorOptions) -> Self {
        Self::with_audio(
            options,
            Box::new(crate::desktop_audio::Microphone),
            Box::new(crate::desktop_audio::Speaker),
        )
    }

    /// As [`Self::new`], with `capture` and `playback` for its microphone, which it never opens, and its speaker.
    pub fn with_audio(
        options: SpectatorOptions,
        capture: Box<dyn capture::Capture>,
        playback: Box<dyn speaker::Playback>,
    ) -> Self {
        let mut overrides = Overrides::default();
        overrides.server = options.server.clone();
        overrides.token = options.token.clone();
        let store = Store::new(None);
        let identity = Identity::new(&store, crate::identity::host_name().as_deref(), overrides);
        let mut conversation = Conversation::new(&identity.name);
        // The spectator follows live speech, and is never played what it missed.
        conversation.follow_live_speech_only();

        // The connection enqueues a `Connect` action when a server is given,
        // so it must be created before `join_space` queues a `Send(Join)`:
        // the `Connect` is performed first, opening the client, and the
        // `Send(Join)` that follows then reaches the server.
        let mut connection = Connection::new(
            identity,
            store,
            Box::new(()),
            capture,
            playback,
            &mut conversation,
        );
        // The spectator's audience sees no interface, so it hears no cues; see `design/spectator.md`.
        connection.feedback().mute_sounds();

        // If a space is given, join it directly, as the lobby's accept would.
        if let Some(space) = &options.space {
            conversation.join_space(space.clone());
        }

        // The presence hook writes each relayed presence into the shared map.
        let presence = Arc::new(Mutex::new(BTreeMap::new()));
        let hook = presence.clone();
        connection.watch_presence(move |message| match message {
            ServerMessage::Presence {
                source,
                head,
                left,
                right,
                hands,
            } => {
                hook.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(
                        *source,
                        ParticipantPresence {
                            head: *head,
                            left: *left,
                            right: *right,
                            hands: hands.as_deref().copied(),
                        },
                    );
            }
            _ => unreachable!("the connection hands its presence hook only presence"),
        });

        Self {
            conversation,
            connection,
            presence,
            follow: options.follow,
            following: None,
            following_name: None,
            workers: Workers::default(),
            scene: Scene::new(),
            swarm: SwarmAnimation::new(),
            agent_home: tomori_scene::anchors::Anchor::AGENT,
            home_ring: tomori_scene::anchors::HomeRing::default(),
            sequence: 0,
            environment: Environment::initial(&[]),
            sceneries: Sceneries::empty(),
            last_poll: Instant::now(),
            last_caption: None,
        }
    }

    /// Draw `sceneries`, as the application that runs the spectator registers them, such as Tomori's own: its `Hello` lists their environments, a space in one is shown in it, and `TOMORI_ENVIRONMENT` chooses among them outside a space, as on the desktop window. See `design/environments.md`.
    pub fn with_sceneries(mut self, sceneries: Sceneries) -> Self {
        self.conversation.set_drawable(sceneries.environments());
        self.conversation.set_over_grid(sceneries.over_grid());
        self.conversation
            .set_worker_placements(sceneries.worker_placements());
        self.conversation.set_avatars(sceneries.avatars());
        self.conversation
            .set_avatar_failures(sceneries.avatar_failures());
        self.environment = Environment::initial(&sceneries.environments());
        self.sceneries = sceneries;
        self
    }

    /// Where its swarm stands and faces, as the joined space's environment puts it, for the renderer's rings about it.
    pub fn swarm_anchor(&self) -> tomori_scene::anchors::Anchor {
        self.swarm.swarm().anchor()
    }

    /// The ring drawn on the floor at home, as the joined space's environment puts it, which stays there wherever the
    /// agent goes. See `design/environments.md`.
    pub fn home_ring(&self) -> tomori_scene::anchors::HomeRing {
        self.home_ring
    }

    /// The registered avatar shown in place of the swarm, if any, and what it shows, for its scenery to draw. See
    /// `design/avatars.md`.
    pub fn avatar(&self) -> Option<tomori_scene::avatar::Shown<'_>> {
        self.swarm.coordinator.shown()
    }

    /// The carriage from the joined space's environment's own coordinates, in which its scenery draws, into the
    /// space's, for the renderer. See `design/environments.md`.
    pub fn carriage(&self) -> tomori_scene::anchors::Anchor {
        self.conversation.arrangement().carriage
    }

    /// The sceneries the spectator draws, for its renderer.
    pub fn sceneries(&self) -> &Sceneries {
        &self.sceneries
    }

    /// Poll the connection and produce the current frame. Non-blocking.
    pub fn poll(&mut self, aspect: f32) -> Frame {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_poll)
            .as_secs_f32()
            .clamp(0.0, 0.1);
        self.last_poll = now;

        // Process connection events, including presence. The presence hook
        // writes into the shared map during this call.
        self.connection.poll(true, &mut self.conversation);

        // Read the presence captured by the hook.
        let presence = self.presence.lock().unwrap().clone();

        // Determine which participant to follow.
        self.update_following(&presence);

        // Build the frame.
        let frame = self.build_frame(&presence, aspect, dt);

        // The audience hears as the participant it follows, from their head, or from the neutral view, and the others from their heads, all in the space's shared scene, where presence already puts each of them; see `design/spatial-audio.md`.
        let listener = match self.following.and_then(|source| presence.get(&source)) {
            Some(followed) => Pose {
                position: followed.head.position,
                orientation: followed.head.orientation,
            },
            None => Pose {
                position: (CENTER + Vec3::new(0.0, 0.0, 3.0)).to_array(),
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
        };
        self.connection.listen_as(self.following);
        crate::spatial_audio::listen(
            &mut self.connection,
            Some(listener),
            self.swarm.coordinator.voice(),
            &self.conversation,
        );

        // Log when the caption changes, so the follow transition is visible.
        if self.last_caption.as_deref() != Some(frame.caption.as_str()) {
            log::info!("Spectator: {}", frame.caption.replace('\n', " "));
            self.last_caption = Some(frame.caption.clone());
        }
        frame
    }

    /// Pick the participant to follow: by name if given, else the first
    /// available in the space's participant list.
    fn update_following(&mut self, presence: &BTreeMap<u32, ParticipantPresence>) {
        let Some((space, _)) = self.conversation.joined_space() else {
            self.following = None;
            self.following_name = None;
            return;
        };

        let target = self.follow.as_deref();
        let participant = space
            .participants
            .iter()
            .find(|p| target.is_none_or(|t| p.name == t) && presence.contains_key(&p.source));

        match participant {
            Some(p) => {
                self.following = Some(p.source);
                self.following_name = Some(p.name.clone());
            }
            None => {
                // No matching participant with presence yet.
                if self.following.is_some() {
                    // The followed participant left; clear.
                    self.following = None;
                    self.following_name = None;
                } else if target.is_some() {
                    // We're waiting for a specific name that hasn't appeared.
                    self.following = None;
                    self.following_name = target.map(str::to_owned);
                }
            }
        }
    }

    fn build_frame(
        &mut self,
        presence: &BTreeMap<u32, ParticipantPresence>,
        aspect: f32,
        dt: f32,
    ) -> Frame {
        self.sequence += 1;

        // Get the followed participant's presence.
        let (head, controllers, hands) = self
            .following
            .and_then(|source| presence.get(&source))
            .map(|p| {
                (
                    Some(p.head),
                    [p.left, p.right],
                    p.hands.unwrap_or([None, None]),
                )
            })
            .unwrap_or((None, [None, None], [None, None]));

        // Build the camera matrices from the head pose.
        let (projection, head_matrix, caption) = match head {
            Some(head) => {
                let quat = Quat::from_array(head.orientation);
                let position = Vec3::from_array(head.position);
                let head_matrix = Mat4::from_rotation_translation(quat, position);
                // The view-projection: a perspective frustum from the head,
                // right-handed, -Z forward, matching the XR convention.
                let projection =
                    Mat4::perspective_rh(1.4, aspect, 0.05, 50.0) * head_matrix.inverse();
                let caption = self.caption_text(&self.following_name);
                (projection, head_matrix, caption)
            }
            None => {
                // No presence yet: a neutral view looking at the scene center.
                let head_matrix = Mat4::from_translation(CENTER);
                let projection = Mat4::perspective_rh(1.4, aspect, 0.05, 50.0)
                    * Mat4::from_translation(CENTER + Vec3::new(0.0, 0.0, 3.0));
                let caption = self.caption_text(&self.following_name);
                (projection, head_matrix, caption)
            }
        };

        // The swarm and the workers stand where the space's environment puts them, by the same rule as the headset. See `design/spectator.md`.
        let arrangement = self.conversation.arrangement();
        // It appears at a new home, as a space opens or changes environment, as the headset's does.
        self.swarm.appear = arrangement.home != self.agent_home;
        self.agent_home = arrangement.home;
        self.swarm.anchor = arrangement.agent;
        self.swarm.head = head.map(|head| Vec3::from_array(head.position));
        // The avatar the space's backend names, by the same rule as the headset. See `design/avatars.md`.
        self.swarm.coordinator.choose(self.conversation.avatar());
        // Use the same activity-driven swarm simulation as the headset.
        self.swarm
            .update(self.conversation.activity(), self.conversation.speech(), dt);
        let agents: Vec<[f32; 3]> = self
            .swarm
            .swarm()
            .agents
            .iter()
            .map(|agent| agent.position.to_array())
            .collect();

        let environment = self.environment();
        // Worker transitions depart from the animated coordinator, as on XR.
        let (root, workspace) = self.workers.update(self.conversation.joined_space());
        self.scene
            .set_coordinator_samples(self.swarm.coordinator.departures());
        // Its satellites orbit the agent while it is away, as on the headset.
        let bounds = self.swarm.coordinator.bounds();
        self.scene
            .set_agent(arrangement.away().then_some((bounds.centre, bounds.radius)));
        self.scene.seat(&arrangement.clusters, &arrangement.layouts);
        self.home_ring = arrangement.home_ring();
        self.scene.arrange_as(
            arrangement.workers,
            arrangement.audience,
            arrangement.extent,
            arrangement.formation,
            arrangement.placement,
        );
        self.scene.update(root, workspace, dt);

        // Convert protocol poses to controller poses for the frame.
        let to_pose = |p: Option<tomori_protocol::Pose>| {
            p.map(|p| Pose {
                position: p.position,
                orientation: p.orientation,
            })
        };

        Frame {
            sequence: self.sequence,
            environment,
            projection: projection.to_cols_array(),
            scene_to_space: Mat4::IDENTITY.to_cols_array(),
            head: head_matrix.to_cols_array(),
            view_aspect: aspect,
            agents,
            controller: None,
            controllers: [to_pose(controllers[0]), to_pose(controllers[1])],
            hands,
            attracting: false,
            caption,
            recording_ms: None,
            interaction: self.swarm.coordinator.visual(),
            worker_cubes: self.scene.cubes().to_vec(),
            worker_stations: self.scene.stations().to_vec(),
            workspace: workspace.clone(),
            presentation_page: 0,
            presentation_content_page: 0,
        }
    }

    fn caption_text(&self, following: &Option<String>) -> String {
        match self.conversation.mode() {
            Mode::Servers => "SPECTATOR\nConnecting to server...".to_string(),
            // Who to follow is chosen only once in the space.
            Mode::Lobby => "SPECTATOR\nJoining space...".to_string(),
            Mode::Space => match following {
                Some(name) => format!("SPECTATOR\nFollowing {name}"),
                None => "SPECTATOR\nIn space, waiting for a participant...".to_string(),
            },
        }
    }
}

impl Feed {
    /// The scenery to show: the joined space's environment, as everyone in it sees it, or the grid when this build cannot draw that one at the version the space's state names, by the rule the headset follows; the spectator's conversation draws Tomori's own, which its Hello lists. See `design/environments.md`.
    fn environment(&self) -> Environment {
        match self.conversation.joined_space() {
            Some((space, _)) if !space.environment.is_empty() => {
                self.conversation.environment_of(space)
            }
            _ => self.environment,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_choice::{Choice, Source};
    use crate::test_support::{Nowhere, Server, Silent};
    use tomori_client::{ConnectionState, Event};
    use tomori_protocol::{Activity, ClientMessage, SpaceState};

    /// A spectator drawing a scenery for [`tomori_scene::test_support::SKY`] and presenting the GitHub prototype's
    /// stand-in over the grid, as an application registers its environments.
    fn feed(options: SpectatorOptions) -> Feed {
        let mut sceneries = tomori_render::scenery::test::Counted::default()
            .sceneries(tomori_scene::test_support::SKY.id());
        sceneries.present_over_grid(tomori_scene::test_support::GITHUB.id(), 1);
        Feed::with_audio(options, Box::new(Silent), Box::new(Nowhere)).with_sceneries(sceneries)
    }

    /// Poll `feed` until `done` says so, returning the last frame.
    fn until(feed: &mut Feed, mut done: impl FnMut(&Feed, &Frame) -> bool) -> Frame {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let frame = feed.poll(16.0 / 9.0);
            if done(feed, &frame) {
                return frame;
            }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn pose(x: f32) -> tomori_protocol::Pose {
        tomori_protocol::Pose {
            position: [x, 1.6, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn presence(right: Option<tomori_protocol::Pose>) -> ClientMessage {
        ClientMessage::Presence {
            head: pose(0.5),
            left: None,
            right,
            hands: Some(Box::new([None, None])),
            carrying: None,
        }
    }

    #[test]
    fn the_spectator_follows_a_participant_by_name_from_their_head() {
        let server = Server::start();
        let bea = server.remote("Bea", Some(server.owner("Bea")));
        let space = bea.create_space("Studio", || {}).id;
        let mut feed = feed(SpectatorOptions {
            server: Some(server.address()),
            space: Some(space.clone()),
            follow: Some("Bea".into()),
            token: None,
        });
        // Until Bea is seen, the spectator waits for her, looking at the scene.
        let frame = until(&mut feed, |feed, _| {
            feed.conversation.joined_space().is_some()
        });
        assert_eq!(frame.controllers, [None, None]);
        assert_eq!(feed.following, None);
        assert_eq!(feed.following_name.as_deref(), Some("Bea"));
        bea.send(presence(Some(pose(0.7))));
        let frame = until(&mut feed, |feed, _| feed.following.is_some());
        assert_eq!(frame.caption, "SPECTATOR\nFollowing Bea");
        assert_eq!(frame.head[12], 0.5);
        assert!(frame.controllers[crate::controller::RIGHT].is_some());
        // The followed participant's right controller stands in for the spectator's marker.
        tomori_render::test_gpu::with(|gpu| {
            let mut renderer = crate::graphics::Renderer::new(
                &gpu.device,
                &gpu.queue,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                64,
                48,
            );
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spectator"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 48,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            crate::spectator_view::SpectatorView::new(feed).render(
                crate::spectator_view::RenderInputs {
                    device: &gpu.device,
                    queue: &gpu.queue,
                    renderer: &mut renderer,
                    swarm: &mut Swarm::default(),
                    target: target.create_view(&Default::default()),
                    aspect: 4.0 / 3.0,
                },
            );
        });
    }

    #[test]
    fn the_spectator_hears_from_the_participant_it_follows_and_the_others_where_they_are_after_travelling()
     {
        use tomori_scene::{arrival::HOME, locomotion::Rig};
        let server = Server::start();
        let bea = server.remote("Bea", Some(server.owner("Bea")));
        let space = bea.create_space("Studio", || {}).id;
        let cy = server.remote("Cy", Some(server.owner("Cy")));
        cy.send(ClientMessage::Join {
            space: space.clone(),
        });
        cy.space(|| {}, |state| (state.id == space).then_some(()));
        let mut feed = feed(SpectatorOptions {
            server: Some(server.address()),
            space: Some(space),
            follow: Some("Bea".into()),
            token: None,
        });
        until(&mut feed, |feed, _| {
            feed.conversation.joined_space().is_some_and(|(space, _)| {
                space
                    .participants
                    .iter()
                    .any(|participant| participant.name == "Cy")
            })
        });
        let (space, _) = feed.conversation.joined_space().unwrap();
        let source = |name: &str| {
            space
                .participants
                .iter()
                .find(|participant| participant.name == name)
                .unwrap()
                .source
        };
        let cy_source = source("Cy");
        // Bea teleported 12 m away onto a ledge 3 m up, and turned; her client sends her head there, in the shared scene.
        let mut travelled = Rig::default();
        travelled.teleport(Vec3::new(4.0, 3.0, -11.0), HOME);
        travelled.turn(1.0, travelled.matrix().transform_point3(HOME));
        let head = travelled.matrix().transform_point3(HOME);
        let orientation = Quat::from_rotation_y(1.0).to_array();
        bea.send(ClientMessage::Presence {
            head: tomori_protocol::Pose {
                position: head.to_array(),
                orientation,
            },
            left: None,
            right: None,
            hands: None,
            carrying: None,
        });
        // Cy travelled to stand 1 m in front of her, as she now faces.
        let ahead = head + Quat::from_array(orientation) * -Vec3::Z;
        cy.send(ClientMessage::Presence {
            head: tomori_protocol::Pose {
                position: ahead.to_array(),
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            left: None,
            right: None,
            hands: None,
            carrying: None,
        });
        let frame = until(&mut feed, |feed, _| {
            feed.following.is_some() && feed.connection.placed().position(cy_source).is_some()
        });
        // The audience sees and hears from where she stands.
        assert!(Vec3::new(frame.head[12], frame.head[13], frame.head[14]).distance(head) < 1e-4);
        let placed = feed.connection.placed().clone();
        let listener = placed.listener.expect("the audience hears from her head");
        assert_eq!(listener.position, head.to_array());
        assert_eq!(listener.orientation, orientation);
        // Cy is heard where their presence says, 1 m in front of her, rather than seated around the space's home.
        assert_eq!(placed.position(cy_source), Some(ahead.to_array()));
        // Bea is the listener, so she is not also a voice.
        assert_eq!(placed.sources.len(), 2);
    }

    #[test]
    fn without_a_name_the_spectator_follows_the_first_participant_it_sees_until_they_leave() {
        let server = Server::start();
        let bea = server.remote("Bea", Some(server.owner("Bea")));
        let space = bea.create_space("Studio", || {}).id;
        let mut feed = feed(SpectatorOptions {
            server: Some(server.address()),
            space: Some(space),
            follow: None,
            token: None,
        });
        let frame = until(&mut feed, |feed, _| {
            feed.conversation.joined_space().is_some()
        });
        assert_eq!(
            frame.caption,
            "SPECTATOR\nIn space, waiting for a participant..."
        );
        bea.send(presence(None));
        let frame = until(&mut feed, |feed, _| feed.following.is_some());
        assert_eq!(frame.caption, "SPECTATOR\nFollowing Bea");
        // Bea leaves the space: the spectator lets go of her.
        bea.send(ClientMessage::Leave);
        until(&mut feed, |feed, _| feed.following.is_none());
        assert_eq!(feed.following_name, None);
    }

    #[test]
    fn frames_follow_the_joined_spaces_activity() {
        let mut feed = feed(SpectatorOptions::default());
        feed.conversation
            .connect(Choice::new("Test", "127.0.0.1:9", Source::Override));
        feed.conversation
            .apply(Event::State(ConnectionState::Connected {
                source: 1,
                server: "Test".into(),
                role: tomori_protocol::ConnectionRole::Visitor,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
                name: None,
            }));
        let mut space = SpaceState {
            id: "space".into(),
            title: "Test".into(),
            activity: Activity::Running,
            ..Default::default()
        };
        feed.conversation
            .apply(Event::Message(ServerMessage::Space {
                state: Box::new(space.clone()),
            }));
        let initial = feed.build_frame(&BTreeMap::new(), 16.0 / 9.0, 1.0 / 60.0);
        assert_eq!(initial.interaction.mode, interaction::Mode::Working);
        assert!(initial.interaction.working);

        space.activity = Activity::Idle;
        feed.conversation
            .apply(Event::Message(ServerMessage::Space {
                state: Box::new(space),
            }));
        let mut frame = initial.clone();
        for _ in 0..120 {
            frame = feed.build_frame(&BTreeMap::new(), 16.0 / 9.0, 1.0 / 60.0);
        }
        assert_eq!(frame.interaction.mode, interaction::Mode::Ready);
        assert!(!frame.interaction.working);
        assert_ne!(frame.agents, initial.agents);
    }

    #[test]
    fn frames_show_the_joined_space_s_environment_as_it_changes() {
        let mut feed = feed(SpectatorOptions::default());
        feed.environment = tomori_scene::test_support::SKY;
        feed.conversation
            .connect(Choice::new("Test", "127.0.0.1:9", Source::Override));
        feed.conversation
            .apply(Event::State(ConnectionState::Connected {
                source: 1,
                server: "Test".into(),
                role: tomori_protocol::ConnectionRole::Visitor,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
                name: None,
            }));
        let versioned = |feed: &mut Feed, environment: &str, version: Option<u32>| {
            feed.conversation
                .apply(Event::Message(ServerMessage::Space {
                    state: Box::new(SpaceState {
                        id: "space".into(),
                        environment: environment.into(),
                        environment_version: version,
                        ..Default::default()
                    }),
                }));
            feed.build_frame(&BTreeMap::new(), 16.0 / 9.0, 1.0 / 60.0)
                .environment
        };
        let frame = |feed: &mut Feed, environment: &str| versioned(feed, environment, None);
        // An owner's change reaches the spectator as it does everyone in the space.
        assert_eq!(frame(&mut feed, "minimal"), Environment::MINIMAL);
        assert_eq!(frame(&mut feed, "sky"), tomori_scene::test_support::SKY);
        // One this build cannot draw shows the grid, and a server that names none keeps the spectator's own.
        assert_eq!(frame(&mut feed, "observe"), Environment::MINIMAL);
        assert_eq!(frame(&mut feed, ""), tomori_scene::test_support::SKY);
        // At the version the spectator draws it at, Sky is drawn, and at another the grid, never its own version.
        assert_eq!(
            versioned(&mut feed, "sky", Some(1)),
            tomori_scene::test_support::SKY
        );
        assert_eq!(versioned(&mut feed, "sky", Some(2)), Environment::MINIMAL);
    }

    #[test]
    fn the_spectator_stands_the_swarm_and_workers_where_the_headset_does() {
        use tomori_scene::anchors::{Anchor, Placing, WorkerPlacement};
        struct Ahead;
        impl WorkerPlacement for Ahead {
            fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
                origin.place(Vec3::new(worker.slot as f32, 1.5, -2.0))
            }
        }
        let mut sceneries = tomori_render::scenery::test::Counted::default()
            .sceneries(tomori_scene::test_support::SKY.id());
        sceneries.set_worker_placement(tomori_scene::test_support::SKY.id(), Ahead);
        let mut feed = Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        )
        .with_sceneries(sceneries);
        feed.conversation
            .connect(Choice::new("Test", "127.0.0.1:9", Source::Override));
        feed.conversation
            .apply(Event::State(ConnectionState::Connected {
                source: 1,
                server: "Test".into(),
                role: tomori_protocol::ConnectionRole::Visitor,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
                name: None,
            }));
        let agent = tomori_protocol::Pose {
            position: [3.0, 1.6, -5.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let workers = tomori_protocol::Pose {
            position: [0.0, 0.0, -4.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        feed.conversation
            .apply(Event::Message(ServerMessage::Space {
                state: Box::new(SpaceState {
                    id: "space".into(),
                    environment: "sky".into(),
                    anchors: tomori_protocol::Anchors::new()
                        .with_agent(agent)
                        .with_workers(workers),
                    workers: vec![tomori_protocol::Worker {
                        id: "ses_a".into(),
                        parent: None,
                        title: "Review".into(),
                        state: tomori_protocol::WorkerState::Running,
                    }],
                    ..Default::default()
                }),
            }));
        let frame = feed.build_frame(&BTreeMap::new(), 16.0 / 9.0, 1.0 / 60.0);
        assert_eq!(feed.swarm_anchor(), Anchor::from_pose(&agent).unwrap());
        // As the headset's frame step places it, by the same placement about the same anchor.
        assert_eq!(frame.worker_stations[0].position, [0.0, 1.5, -6.0]);
    }

    #[test]
    fn the_spectator_shows_the_avatar_the_space_s_backend_names_as_the_headset_does() {
        use tomori_scene::avatar::{Avatar, Input};
        /// A figure at its anchor, heard from above it, sending workers from beside it.
        struct Figure(tomori_scene::anchors::Anchor);
        impl Avatar for Figure {
            fn update(&mut self, input: &Input) {
                self.0 = input.anchor;
            }
            fn voices(&self, voices: &mut Vec<[f32; 3]>) {
                voices.push((self.0.position + Vec3::Y).to_array());
            }
            fn departures(&self, departures: &mut Vec<[f32; 3]>) {
                departures.push((self.0.position + Vec3::X).to_array());
            }
            fn bounds(&self) -> tomori_scene::arrival_motion::Sphere {
                tomori_scene::arrival_motion::Sphere {
                    centre: self.0.position,
                    radius: 1.0,
                }
            }
        }
        let mut sceneries = Sceneries::empty();
        sceneries.set_avatar(
            "figure",
            |_| Box::new(Figure(tomori_scene::anchors::Anchor::AGENT)),
            tomori_render::scenery::test::blank,
        );
        let mut feed = Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        )
        .with_sceneries(sceneries);
        feed.conversation
            .connect(Choice::new("Test", "127.0.0.1:9", Source::Override));
        feed.conversation
            .apply(Event::State(ConnectionState::Connected {
                source: 1,
                server: "Test".into(),
                role: tomori_protocol::ConnectionRole::Visitor,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
                name: None,
            }));
        let state = |avatar: &str| {
            Event::Message(ServerMessage::Space {
                state: Box::new(SpaceState {
                    id: "space".into(),
                    environment: "minimal".into(),
                    avatar: Some(avatar.into()),
                    ..Default::default()
                }),
            })
        };
        feed.conversation.apply(state("figure"));
        let frame = feed.build_frame(&BTreeMap::new(), 16.0 / 9.0, 1.0 / 60.0);
        // The figure shows in place of the swarm, which is not drawn.
        assert_eq!(feed.avatar().unwrap().avatar, "figure");
        assert!(frame.agents.is_empty());
        // Its audience hears the agent from it.
        assert_eq!(
            feed.swarm.coordinator.voice(),
            (CENTER + Vec3::Y).to_array()
        );
        // Workers fly from it and back to it.
        assert_eq!(
            feed.swarm.coordinator.departures(),
            [(CENTER + Vec3::X).to_array()]
        );
        assert_eq!(feed.swarm.coordinator.bounds().radius, 1.0);
        tomori_render::test_gpu::with(|gpu| {
            let mut renderer = crate::graphics::Renderer::with_sceneries(
                &gpu.device,
                &gpu.queue,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                64,
                48,
                1,
                feed.sceneries(),
            );
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spectator avatar"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 48,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let mut swarm = Swarm::default();
            let mut view = crate::spectator_view::SpectatorView::new(feed);
            let mut render = |swarm: &mut Swarm| {
                view.render(crate::spectator_view::RenderInputs {
                    device: &gpu.device,
                    queue: &gpu.queue,
                    renderer: &mut renderer,
                    swarm,
                    target: target.create_view(&Default::default()),
                    aspect: 4.0 / 3.0,
                })
            };
            // The view draws no swarm while the figure shows.
            render(&mut swarm);
            assert!(swarm.agents.is_empty());
        });
        // A space whose backend names an avatar the spectator lacks shows the swarm.
        let mut plain = Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        );
        plain
            .conversation
            .connect(Choice::new("Test", "127.0.0.1:9", Source::Override));
        plain
            .conversation
            .apply(Event::State(ConnectionState::Connected {
                source: 1,
                server: "Test".into(),
                role: tomori_protocol::ConnectionRole::Visitor,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
                name: None,
            }));
        plain.conversation.apply(state("figure"));
        let frame = plain.build_frame(&BTreeMap::new(), 16.0 / 9.0, 1.0 / 60.0);
        assert!(plain.avatar().is_none());
        assert_eq!(frame.agents.len(), crate::swarm::AGENT_COUNT);
    }

    #[test]
    fn hello_lists_the_environments_the_spectator_draws() {
        use tomori_client::connection::Session;
        let feed = feed(SpectatorOptions::default());
        let listed: Vec<(String, u32)> = feed
            .conversation
            .environments()
            .unwrap()
            .into_iter()
            .map(|presented| (presented.id, presented.version))
            .collect();
        assert_eq!(
            listed,
            [
                ("minimal".to_owned(), 1),
                ("sky".to_owned(), 1),
                ("github".to_owned(), 1)
            ]
        );
        assert!(feed.sceneries().contains("sky"));
        // Until it is given sceneries, it draws and lists the grid alone, and starts with it.
        let plain = Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        );
        assert_eq!(
            plain.conversation.environments(),
            Some(vec![tomori_protocol::EnvironmentVersion::new("minimal", 1)])
        );
        assert_eq!(plain.environment, Environment::MINIMAL);
        assert_eq!(plain.sceneries().environments(), [Environment::MINIMAL]);
    }

    #[test]
    fn swarm_animates_with_space_activity() {
        let mut animation = SwarmAnimation::new();
        let initial = animation.swarm().agents[0].position;
        for _ in 0..120 {
            animation.update(
                interaction::SessionActivity::Working,
                Default::default(),
                1.0 / 60.0,
            );
        }
        assert_eq!(animation.swarm().visual.mode, interaction::Mode::Working);
        assert!(animation.swarm().visual.working);
        assert!(animation.swarm().agents[0].position.distance(initial) > 0.01);

        // The agent speaks while the speech the spectator plays does, though its session is idle.
        for _ in 0..60 {
            animation.update(
                interaction::SessionActivity::Ready,
                interaction::SpeechActivity {
                    playing: true,
                    level: 0.5,
                    ..Default::default()
                },
                1.0 / 60.0,
            );
        }
        assert_eq!(animation.swarm().visual.mode, interaction::Mode::Speaking);
        assert!(animation.swarm().visual.level > 0.0);

        for _ in 0..120 {
            animation.update(
                interaction::SessionActivity::Ready,
                Default::default(),
                1.0 / 60.0,
            );
        }
        assert_eq!(animation.swarm().visual.mode, interaction::Mode::Ready);
        assert!(!animation.swarm().visual.working);
        assert!(animation.swarm().visual.time > 0.0);
    }

    #[test]
    fn swarm_uses_the_same_simulation_as_the_headset() {
        let mut animation = SwarmAnimation::new();
        let mut signals = interaction::Signals::default();
        let mut headset = Swarm::default();
        let speaking = interaction::SpeechActivity {
            playing: true,
            level: 0.4,
            ..Default::default()
        };
        for (activity, speech) in [
            (interaction::SessionActivity::Working, Default::default()),
            (interaction::SessionActivity::Ready, speaking),
            (interaction::SessionActivity::Waiting, Default::default()),
            (interaction::SessionActivity::Ready, Default::default()),
        ] {
            for _ in 0..80 {
                let dt = 1.0 / 60.0;
                animation.update(activity, speech, dt);
                let visual = signals.update(
                    interaction::Inputs {
                        session: activity,
                        speech,
                        ..Default::default()
                    },
                    dt,
                );
                headset.set_visual(visual, dt);
                headset.step(dt, None);
            }
        }
        assert_eq!(animation.swarm().visual, headset.visual);
        for (spectator, headset) in animation.swarm().agents.iter().zip(&headset.agents) {
            assert_eq!(spectator.position, headset.position);
        }
    }
}
