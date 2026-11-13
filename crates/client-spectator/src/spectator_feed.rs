// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spectator's live feed: connects to a space the same way the headset does
//! (`Conversation` + `Connection`), tracks participant presence relayed by the
//! server, chooses whom to follow, and runs the frame step a participant's
//! client runs, as a watcher, from the followed participant's head. See
//! `design/spectator.md`.
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
    identity::{Identity, Overrides, Store},
    speaker,
};
use glam::{Mat4, Quat, Vec3};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tomori_client::Hearing;
use tomori_protocol::{Hand, ServerMessage};
use tomori_render::scenery::Sceneries;
use tomori_scene::scene::{Frame, Step};

/// Where the spectator sees and hears from with nobody to follow, facing −Z: where a participant joining the space
/// stands, at home with their rig unmoved, at standing eye height, as their headset starts. An environment's arrival is
/// carried to home, so this is where everyone joins in every environment; anchors that spread participants out as they
/// arrive (#79) would move it, once they exist.
const NEUTRAL: Vec3 = tomori_scene::arrival::HOME;

/// The spectator's vertical field of view, in radians.
const FIELD_OF_VIEW: f32 = 1.4;

/// A participant's latest presence report, as relayed by the server.
#[derive(Clone)]
struct ParticipantPresence {
    head: tomori_protocol::Pose,
    left: Option<tomori_protocol::Pose>,
    right: Option<tomori_protocol::Pose>,
    hands: Option<[Option<Hand>; 2]>,
}

/// The options a spectator is started with: [`Self::new`]'s, none, changed field by field, so that options added later
/// break no application that sets its own.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SpectatorOptions {
    /// A server address to connect to straight away.
    pub server: Option<String>,
    /// A space id to join, as printed by `tomori discover`.
    pub space: Option<String>,
    /// A participant name to follow, or the first available.
    pub follow: Option<String>,
    /// A token for the server, for a server that refuses visitors. See `design/security.md`.
    pub token: Option<String>,
    /// Whose voices it plays: everyone's, by default, the agent's speech alone, or none. See `design/spectator.md`.
    pub voices: Hearing,
}

impl SpectatorOptions {
    /// No server, space, participant or token, and every voice played.
    pub fn new() -> Self {
        Self::default()
    }
}

/// One frame of the spectator's view: what the frame step draws, as a watcher, with the camera's view and projection,
/// the followed participant's controllers and hands, which the spectator draws as theirs, and its caption, which it
/// logs.
pub struct Watched<'a> {
    pub frame: Frame<'a>,
    pub projection: Mat4,
    pub controllers: [Option<Pose>; 2],
    pub hands: [Option<Hand>; 2],
    pub caption: String,
}

/// The spectator's connection to a space. Tracks participant presence, and
/// runs the frame step from the followed participant's view.
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
    /// The frame step a participant's client runs, as a watcher runs it. See `design/spectator.md`.
    step: Step,
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
        // And watches from where it chooses, standing nowhere anyone arrives. See `design/spectator.md`.
        conversation.choose_own_arrival();

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
        connection.hear(options.voices);

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
            step: Step::default(),
            sceneries: Sceneries::empty(),
            last_poll: Instant::now(),
            last_caption: None,
        }
    }

    /// Whose voices it plays, as its connection was last told.
    pub fn voices(&self) -> Hearing {
        self.connection.hearing()
    }

    /// Play the next voices from now on, everyone's, then the agent's alone, then none, and everyone's again, as the
    /// window's V key chooses, returning them. See `design/spectator.md`.
    pub fn next_voices(&mut self) -> Hearing {
        let voices = self.voices().next();
        self.connection.hear(voices);
        log::info!("Spectator: voices {}", voices.name());
        voices
    }

    /// Draw `sceneries`, as the application that runs the spectator registers them, such as Tomori's own: its `Hello` lists their environments and a space in one is shown in it, as for a participant. The client sides registered with them run as a watcher's, drawing what they show. See `design/environments.md` and `design/spectator.md`.
    pub fn with_sceneries(mut self, sceneries: Sceneries) -> Self {
        self.conversation.set_drawable(sceneries.environments());
        self.conversation.set_client_sides(sceneries.client_sides());
        self.conversation.set_over_grid(sceneries.over_grid());
        self.conversation
            .set_worker_placements(sceneries.worker_placements());
        self.conversation.set_avatars(sceneries.avatars());
        self.conversation
            .set_avatar_failures(sceneries.avatar_failures());
        self.sceneries = sceneries;
        self
    }

    /// The sceneries the spectator draws, for its renderer.
    pub fn sceneries(&self) -> &Sceneries {
        &self.sceneries
    }

    /// How many intents the environment's client sides have asked, all dropped, since a spectator asks nothing.
    pub fn dropped_intents(&self) -> usize {
        self.step.dropped()
    }

    /// Poll the connection and produce the current frame, for a view `aspect` times as wide as it is tall. Non-blocking.
    pub fn poll(&mut self, aspect: f32) -> Watched<'_> {
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

        self.watch(&presence, aspect, dt, now)
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

    /// One frame, `dt` seconds after the last, at `now`: the frame step as a watcher, from the followed participant's
    /// head in `presence`, or the neutral view, heard from there too.
    fn watch(
        &mut self,
        presence: &BTreeMap<u32, ParticipantPresence>,
        aspect: f32,
        dt: f32,
        now: Instant,
    ) -> Watched<'_> {
        let followed = self.following.and_then(|source| presence.get(&source));
        // The camera: the followed participant's head, where their presence puts it in the space's shared scene, or,
        // with nobody to follow, where a spectator arrives, looking ahead.
        let camera = match followed {
            Some(followed) => Mat4::from_rotation_translation(
                Quat::from_array(followed.head.orientation),
                Vec3::from_array(followed.head.position),
            ),
            None => self.neutral(),
        };
        let to_pose = |pose: Option<tomori_protocol::Pose>| {
            pose.map(|pose| Pose {
                position: pose.position,
                orientation: pose.orientation,
            })
        };
        let (controllers, hands) = followed.map_or(([None, None], [None, None]), |followed| {
            (
                [to_pose(followed.left), to_pose(followed.right)],
                followed.hands.unwrap_or([None, None]),
            )
        });
        let caption = self.caption_text();
        // Log when the caption changes, so the follow transition is visible.
        if self.last_caption.as_deref() != Some(caption.as_str()) {
            log::info!("Spectator: {}", caption.replace('\n', " "));
            self.last_caption = Some(caption.clone());
        }
        let frame = self.step.watch(
            camera,
            &mut self.conversation,
            self.connection.feedback(),
            now,
            dt,
        );
        // The audience hears as the participant it follows, from their head, or from the neutral view, and the others
        // from their heads, all in the space's shared scene, where presence already puts each of them; see
        // `design/spatial-audio.md`.
        self.connection.listen_as(self.following);
        crate::spatial_audio::listen(
            &mut self.connection,
            frame.listener,
            frame.agent,
            &self.conversation,
        );
        Watched {
            frame,
            // The view-projection: a perspective frustum from the camera, right-handed, −Z forward, as in XR.
            projection: tomori_scene::depth::perspective(FIELD_OF_VIEW, aspect) * camera.inverse(),
            controllers,
            hands,
            caption,
        }
    }

    /// Where it sees and hears from while it follows nobody: at standing eye height above the spawn point the joined
    /// space's environment tags for spectators, or else its first untagged, facing its way, as a room arriving there
    /// would; or, with none, home, facing −Z, where a participant joins as a headset starts. See `design/spectator.md`.
    fn neutral(&self) -> Mat4 {
        let rig = self
            .conversation
            .spectator_spawn()
            .map_or(tomori_scene::locomotion::Rig::default(), |spawn| {
                tomori_scene::locomotion::Rig::arriving(&spawn)
            });
        rig.matrix() * Mat4::from_translation(NEUTRAL)
    }

    fn caption_text(&self) -> String {
        match self.conversation.mode() {
            Mode::Servers => "SPECTATOR\nConnecting to server...".to_string(),
            // Who to follow is chosen only once in the space.
            Mode::Lobby => "SPECTATOR\nJoining space...".to_string(),
            Mode::Space => match &self.following_name {
                Some(name) => format!("SPECTATOR\nFollowing {name}"),
                None => "SPECTATOR\nIn space, waiting for a participant...".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_choice::{Choice, Source};
    use crate::test_support::{Nowhere, Server, Silent};
    use tomori_client::{ConnectionState, Event};
    use tomori_protocol::{Activity, ClientMessage, ConnectionRole, SpaceState};
    use tomori_scene::{environment::Environment, swarm::CENTER};

    /// A spectator drawing a scenery for [`tomori_scene::test_support::SKY`] and presenting the GitHub prototype's
    /// stand-in over the grid, as an application registers its environments.
    fn feed(options: SpectatorOptions) -> Feed {
        let mut sceneries = tomori_render::scenery::test::Counted::default()
            .sceneries(tomori_scene::test_support::SKY.id());
        sceneries.present_over_grid(tomori_scene::test_support::GITHUB.id(), 1);
        Feed::with_audio(options, Box::new(Silent), Box::new(Nowhere)).with_sceneries(sceneries)
    }

    /// A spectator drawing `sceneries`, told it has connected as `role`, to be shown a space without a server.
    fn connected(sceneries: Sceneries, role: ConnectionRole) -> Feed {
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
                role,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
                name: None,
            }));
        feed
    }

    /// Show `feed` the space `state`, as its server sends it.
    fn show(feed: &mut Feed, state: SpaceState) {
        feed.conversation
            .apply(Event::Message(ServerMessage::Space {
                state: Box::new(state),
            }));
    }

    /// What a frame of the spectator's view showed, kept beyond it.
    struct Seen {
        caption: String,
        projection: Mat4,
        controllers: [Option<Pose>; 2],
        environment: Environment,
        visual: tomori_scene::interaction::Visual,
        agents: Vec<Vec3>,
        anchor: tomori_scene::anchors::Anchor,
        home_ring: tomori_scene::anchors::HomeRing,
        dials: Vec<tomori_scene::dials::Dial>,
        stations: Vec<[f32; 3]>,
        cubes: usize,
        avatar: Option<String>,
        agent: [f32; 3],
        drawn: Option<usize>,
        faces: usize,
        images: Vec<(String, Arc<[u8]>)>,
    }

    impl Seen {
        fn of(watched: &Watched) -> Self {
            let frame = &watched.frame;
            Self {
                caption: watched.caption.clone(),
                projection: watched.projection,
                controllers: watched.controllers,
                environment: frame.scenery.environment,
                visual: frame.swarm.visual,
                agents: frame
                    .swarm
                    .agents
                    .iter()
                    .map(|agent| agent.position)
                    .collect(),
                anchor: frame.swarm.anchor(),
                home_ring: frame.home_ring,
                dials: frame.dials.to_vec(),
                stations: frame
                    .stations
                    .iter()
                    .map(|station| station.position)
                    .collect(),
                cubes: frame.cubes.len(),
                avatar: frame.avatar.as_ref().map(|shown| shown.avatar.to_owned()),
                agent: frame.agent,
                drawn: frame
                    .environment
                    .as_ref()
                    .and_then(|drawn| drawn.downcast_ref::<usize>().copied()),
                faces: frame
                    .projection
                    .iter()
                    .filter(|sheet| sheet.canvas.is_some() && sheet.model.is_some())
                    .count(),
                images: frame.images.clone(),
            }
        }

        /// Where the camera is.
        fn eye(&self) -> Vec3 {
            tomori_render::scenery::eye(self.projection)
        }
    }

    /// One frame of `feed`, `dt` seconds after the last, with nobody to follow.
    fn frame(feed: &mut Feed, dt: f32) -> Seen {
        Seen::of(&feed.watch(&BTreeMap::new(), 16.0 / 9.0, dt, Instant::now()))
    }

    /// The frame once `feed`'s space has grown in, as it does for everyone joining it.
    fn grown(feed: &mut Feed) -> Seen {
        for _ in 0..400 {
            frame(feed, 1.0 / 60.0);
        }
        frame(feed, 1.0 / 60.0)
    }

    /// Poll `feed` until `done` says so, returning the last frame.
    fn until(feed: &mut Feed, mut done: impl FnMut(&Feed) -> bool) -> Seen {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let seen = Seen::of(&feed.poll(16.0 / 9.0));
            if done(feed) {
                return seen;
            }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A target of `width` by `height` to render the spectator's view into, which may be copied from.
    fn target(gpu: &tomori_render::test_gpu::Gpu, width: u32, height: u32) -> wgpu::Texture {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spectator"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Render `view` into `target` with `renderer`.
    fn render(
        gpu: &tomori_render::test_gpu::Gpu,
        view: &mut crate::spectator_view::SpectatorView,
        renderer: &mut crate::graphics::Renderer,
        target: &wgpu::Texture,
    ) {
        view.render(crate::spectator_view::RenderInputs {
            device: &gpu.device,
            queue: &gpu.queue,
            renderer,
            target: target.create_view(&Default::default()),
            aspect: target.width() as f32 / target.height() as f32,
        });
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
            carried: Vec::new(),
            rays: None,
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
            ..Default::default()
        });
        // Until Bea is seen, the spectator waits for her, looking at the scene.
        let seen = until(&mut feed, |feed| feed.conversation.joined_space().is_some());
        assert_eq!(seen.controllers, [None, None]);
        assert_eq!(feed.following, None);
        assert_eq!(feed.following_name.as_deref(), Some("Bea"));
        bea.send(presence(Some(pose(0.7))));
        let seen = until(&mut feed, |feed| feed.following.is_some());
        assert_eq!(seen.caption, "SPECTATOR\nFollowing Bea");
        assert!(seen.eye().distance(Vec3::new(0.5, 1.6, 0.0)) < 1e-4);
        assert!(seen.controllers[crate::controller::RIGHT].is_some());
        // The followed participant's right controller is drawn as theirs.
        tomori_render::test_gpu::with(|gpu| {
            let mut renderer = crate::graphics::Renderer::new(
                &gpu.device,
                &gpu.queue,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                64,
                48,
            );
            let target = target(&gpu, 64, 48);
            let mut view = crate::spectator_view::SpectatorView::new(feed);
            render(&gpu, &mut view, &mut renderer, &target);
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
            ..Default::default()
        });
        until(&mut feed, |feed| {
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
            carried: Vec::new(),
            rays: None,
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
            carried: Vec::new(),
            rays: None,
        });
        let seen = until(&mut feed, |feed| {
            feed.following.is_some() && feed.connection.placed().position(cy_source).is_some()
        });
        // The audience sees and hears from where she stands.
        assert!(seen.eye().distance(head) < 1e-3, "{}", seen.eye());
        let placed = feed.connection.placed().clone();
        let listener = placed.listener.expect("the audience hears from her head");
        assert!(Vec3::from_array(listener.position).distance(head) < 1e-4);
        assert!(
            Quat::from_array(listener.orientation).angle_between(Quat::from_array(orientation))
                < 1e-3
        );
        // Cy is heard where their presence says, 1 m in front of her, rather than seated around the space's home.
        assert_eq!(placed.position(cy_source), Some(ahead.to_array()));
        // Bea is the listener, so she is not also a voice.
        assert_eq!(placed.sources.len(), 2);
    }

    #[test]
    fn the_voices_played_are_those_chosen_and_v_cycles_through_them() {
        assert_eq!(feed(SpectatorOptions::new()).voices(), Hearing::All);
        let mut demonstrating = feed(SpectatorOptions {
            voices: Hearing::Agent,
            ..Default::default()
        });
        assert_eq!(demonstrating.connection.hearing(), Hearing::Agent);
        let cycled = [(); 3].map(|()| demonstrating.next_voices());
        assert_eq!(cycled, [Hearing::Nobody, Hearing::All, Hearing::Agent]);
        // Each is what its connection was told, so that it is what plays.
        demonstrating.next_voices();
        assert_eq!(demonstrating.connection.hearing(), Hearing::Nobody);
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
            ..Default::default()
        });
        let seen = until(&mut feed, |feed| feed.conversation.joined_space().is_some());
        assert_eq!(
            seen.caption,
            "SPECTATOR\nIn space, waiting for a participant..."
        );
        bea.send(presence(None));
        let seen = until(&mut feed, |feed| feed.following.is_some());
        assert_eq!(seen.caption, "SPECTATOR\nFollowing Bea");
        // Bea leaves the space: the spectator lets go of her.
        bea.send(ClientMessage::Leave);
        until(&mut feed, |feed| feed.following.is_none());
        assert_eq!(feed.following_name, None);
    }

    #[test]
    fn its_captions_say_how_it_is_getting_on() {
        let mut feed = feed(SpectatorOptions::default());
        assert_eq!(
            frame(&mut feed, 0.0).caption,
            "SPECTATOR\nConnecting to server..."
        );
        let mut feed = connected(Sceneries::empty(), ConnectionRole::Visitor);
        assert_eq!(frame(&mut feed, 0.0).caption, "SPECTATOR\nJoining space...");
    }

    #[test]
    fn frames_follow_the_joined_spaces_activity() {
        let mut feed = connected(Sceneries::empty(), ConnectionRole::Visitor);
        let mut space = SpaceState {
            id: "space".into(),
            title: "Test".into(),
            activity: Activity::Running,
            ..Default::default()
        };
        show(&mut feed, space.clone());
        let initial = frame(&mut feed, 1.0 / 60.0);
        assert_eq!(
            initial.visual.mode,
            tomori_scene::interaction::Mode::Working
        );
        assert!(initial.visual.working);

        space.activity = Activity::Idle;
        show(&mut feed, space);
        let mut seen = frame(&mut feed, 1.0 / 60.0);
        for _ in 0..120 {
            seen = frame(&mut feed, 1.0 / 60.0);
        }
        assert_eq!(seen.visual.mode, tomori_scene::interaction::Mode::Ready);
        assert!(!seen.visual.working);
        assert_ne!(seen.agents, initial.agents);
    }

    #[test]
    fn frames_carry_the_space_s_timers_as_dials_above_the_agent_as_the_headset_draws_them() {
        let mut feed = connected(Sceneries::empty(), ConnectionRole::Visitor);
        let next = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3 * 3600
            + 300;
        show(
            &mut feed,
            SpaceState {
                id: "space".into(),
                title: "Test".into(),
                timers: vec![tomori_protocol::Timer::new(
                    "standup",
                    "weekdays at 09:00",
                    "when present",
                    Some(next),
                    tomori_protocol::TimerState::Paused,
                )],
                ..Default::default()
            },
        );
        let mut seen = frame(&mut feed, 1.0 / 60.0);
        for _ in 0..30 {
            seen = frame(&mut feed, 1.0 / 20.0);
        }
        assert_eq!(seen.dials.len(), 1);
        let dial = &seen.dials[0];
        assert_eq!(dial.name, "standup");
        assert!(dial.pill.starts_with("Paused 3h 0"), "{}", dial.pill);
        let places = tomori_scene::dials::places(
            1,
            &tomori_scene::dials::Room::open(
                tomori_scene::anchors::Anchor::AGENT,
                tomori_scene::anchors::EYES,
            ),
        );
        assert_eq!(dial.position, places[0].0.to_array());
        // Its cubes among the workers'.
        assert!(seen.cubes > 0);
    }

    #[test]
    fn frames_show_the_joined_space_s_environment_as_it_changes() {
        let mut sceneries = tomori_render::scenery::test::Counted::default()
            .sceneries(tomori_scene::test_support::SKY.id());
        sceneries.present_over_grid(tomori_scene::test_support::GITHUB.id(), 1);
        let mut feed = connected(sceneries, ConnectionRole::Visitor);
        let mut versioned = |environment: &str, version: Option<u32>| {
            show(
                &mut feed,
                SpaceState {
                    id: "space".into(),
                    environment: environment.into(),
                    environment_version: version,
                    ..Default::default()
                },
            );
            grown(&mut feed).environment
        };
        // An owner's change reaches the spectator as it does everyone in the space.
        assert_eq!(versioned("minimal", None), Environment::MINIMAL);
        assert_eq!(versioned("sky", None), tomori_scene::test_support::SKY);
        // One this build cannot draw shows the grid, as does a server that names none, as for a participant.
        assert_eq!(versioned("observe", None), Environment::MINIMAL);
        assert_eq!(versioned("", None), Environment::MINIMAL);
        // At the version the spectator draws it at, Sky is drawn, and at another the grid, never its own version.
        assert_eq!(versioned("sky", Some(1)), tomori_scene::test_support::SKY);
        assert_eq!(versioned("sky", Some(2)), Environment::MINIMAL);
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
        let mut feed = connected(sceneries, ConnectionRole::Visitor);
        let agent = tomori_protocol::Pose {
            position: [3.0, 1.6, -5.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let workers = tomori_protocol::Pose {
            position: [0.0, 0.0, -4.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        show(
            &mut feed,
            SpaceState {
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
            },
        );
        let seen = frame(&mut feed, 1.0 / 60.0);
        assert_eq!(seen.anchor, Anchor::from_pose(&agent).unwrap());
        // Its ring on the floor about the agent's home, as the headset draws it.
        assert_eq!(
            seen.home_ring,
            tomori_scene::anchors::HomeRing {
                centre: glam::Vec3::from_array(agent.position),
                ..tomori_scene::anchors::HomeRing::WHOLE
            }
        );
        // As the headset's frame step places it, by the same placement about the same anchor.
        assert_eq!(seen.stations[0], [0.0, 1.5, -6.0]);
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
        let state = || SpaceState {
            id: "space".into(),
            environment: "minimal".into(),
            avatar: Some("figure".into()),
            ..Default::default()
        };
        let mut feed = connected(sceneries, ConnectionRole::Visitor);
        show(&mut feed, state());
        let seen = frame(&mut feed, 1.0 / 60.0);
        // The figure shows in place of the swarm, which is not drawn.
        assert_eq!(seen.avatar.as_deref(), Some("figure"));
        assert!(seen.agents.is_empty());
        // Its audience hears the agent from it.
        assert_eq!(seen.agent, (CENTER + Vec3::Y).to_array());
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
            let target = target(&gpu, 64, 48);
            let mut view = crate::spectator_view::SpectatorView::new(feed);
            render(&gpu, &mut view, &mut renderer, &target);
        });
        // A space whose backend names an avatar the spectator lacks shows the swarm.
        let mut plain = connected(Sceneries::empty(), ConnectionRole::Visitor);
        show(&mut plain, state());
        let seen = frame(&mut plain, 1.0 / 60.0);
        assert!(seen.avatar.is_none());
        assert_eq!(seen.agents.len(), tomori_scene::swarm::AGENT_COUNT);
    }

    #[test]
    fn the_spectator_runs_and_draws_the_environment_s_client_side_and_asks_nothing() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tomori_scene::client_side::{ClientSide, Input, Output, Picked};
        /// A scenery that keeps what its client side last showed it.
        struct Shown(Arc<AtomicUsize>);
        impl tomori_render::scenery::Scenery for Shown {
            fn animate(
                &mut self,
                _device: &wgpu::Device,
                _queue: &wgpu::Queue,
                drawn: &(dyn std::any::Any + Send + Sync),
            ) {
                self.0
                    .store(*drawn.downcast_ref::<usize>().unwrap(), Ordering::Relaxed);
            }
            fn prepare(&self, _queue: &wgpu::Queue, _eye: usize, _scene_to_clip: Mat4) {}
            fn draw(&self, _pass: &mut wgpu::RenderPass<'_>, _eye: usize) {}
            fn set_reveal(&mut self, _reveal: Option<(Vec3, f32)>) {}
        }
        /// A client side that shows how many updates it has had, keeps the head it was last told, and asks its server
        /// side something every frame.
        struct Counting(usize, Arc<Mutex<Option<Vec3>>>);
        impl ClientSide for Counting {
            fn pick(&self, _ray: &tomori_scene::pointing::Ray) -> Option<Picked> {
                Some(Picked::new("thing", "the thing", 1.0))
            }
            fn update(&mut self, input: Input<'_>) -> Output {
                self.0 += input.updates.len();
                *self.1.lock().unwrap() = input.head;
                Output::new()
                    .with_intent(tomori_protocol::Payload::new(serde_json::json!({})))
                    .drawing(Arc::new(self.0 + 1))
            }
        }
        let animated = Arc::new(AtomicUsize::new(0));
        let mut sceneries = Sceneries::empty();
        let shown = animated.clone();
        sceneries.register(tomori_scene::test_support::SKY.id(), move |_| {
            Box::new(Shown(shown.clone()))
        });
        let head = Arc::new(Mutex::new(None));
        let told = head.clone();
        sceneries.set_client_side(tomori_scene::test_support::SKY.id(), move |_| {
            Box::new(Counting(0, told.clone()))
        });
        // A spectator given an owner's token is no more a participant to its client side.
        let mut feed = connected(sceneries, ConnectionRole::Owner);
        show(
            &mut feed,
            SpaceState {
                id: "space".into(),
                environment: "sky".into(),
                ..Default::default()
            },
        );
        for message in [
            ServerMessage::EnvironmentState {
                environment: "sky".into(),
                version: 1,
                state: tomori_protocol::Payload::new(serde_json::json!({})),
            },
            ServerMessage::EnvironmentEvent {
                environment: "sky".into(),
                version: 1,
                event: tomori_protocol::Payload::new(serde_json::json!({})),
            },
        ] {
            feed.conversation.apply(Event::Message(message));
        }
        feed.conversation.take_actions();
        let seen = grown(&mut feed);
        // With nobody to follow, it sees from where a participant joins, at home at standing eye height facing −Z, where
        // its audience hears, and its client side's head is there.
        assert!(seen.eye().distance(NEUTRAL) < 1e-4, "{}", seen.eye());
        assert!(head.lock().unwrap().unwrap().distance(NEUTRAL) < 1e-4);
        assert_eq!(NEUTRAL, tomori_scene::anchors::EYES);
        let ahead = seen.projection.project_point3(NEUTRAL + Vec3::NEG_Z);
        assert!(ahead.x.abs() < 1e-4 && ahead.y.abs() < 1e-4, "{ahead}");
        // The environment's messages reached its client side, which shows them, and what it asked was dropped.
        assert_eq!(seen.drawn, Some(3));
        assert!(feed.dropped_intents() > 0);
        let actions = feed.conversation.take_actions();
        assert!(!format!("{actions:?}").contains("EnvironmentIntent"));
        // Though its object is there to point at, the spectator points at nothing.
        let ray = tomori_scene::pointing::Ray::new(NEUTRAL, Vec3::NEG_Z).unwrap();
        assert!(Counting(0, head.clone()).pick(&ray).is_some());
        tomori_render::scenery::Scenery::set_reveal(&mut Shown(animated.clone()), None);
        // Its scenery is given what the client side shows, in an offscreen render of the spectator's view.
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
            let target = target(&gpu, 64, 48);
            let mut view = crate::spectator_view::SpectatorView::new(feed);
            render(&gpu, &mut view, &mut renderer, &target);
            assert_eq!(animated.load(Ordering::Relaxed), 3);
        });
    }

    #[test]
    fn the_spectator_draws_the_space_s_walls_and_the_panels_published_on_them_as_a_visitor() {
        use tomori_protocol::{Panel, Place, Projection, TOMORI_WALL, WALL_SLOTS, Wall, WallKind};
        // Rows of a multiple of 256 bytes, as wgpu copies them.
        const WIDTH: u32 = 192;
        const HEIGHT: u32 = 144;
        // A one-pixel PNG, which an image panel shows.
        const PNG: [u8; 70] = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0xf8, 0xdf, 0xc0, 0xf0, 0x1f, 0x00, 0x06, 0x80, 0x02, 0x7f, 0x10, 0x4c,
            0x1b, 0xe1, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let hash = format!("{:064x}", 1);
        // A visitor's spectator in a space whose server projects the Tomori wall ahead, with `panels` on it, holding the
        // image they show if it was `fetched`.
        let watching = |panels: Vec<Panel>, fetched: bool| {
            let mut feed = connected(Sceneries::empty(), ConnectionRole::Visitor);
            show(
                &mut feed,
                SpaceState {
                    id: "space".into(),
                    projection: true,
                    ..Default::default()
                },
            );
            let wall = Wall::new(
                TOMORI_WALL,
                WallKind::Tomori,
                tomori_protocol::Pose::upright_facing([0.0, 1.2, -2.3], [0.0, 1.2, 0.0]),
            );
            feed.conversation
                .apply(Event::Message(ServerMessage::Projection {
                    projection: Box::new(
                        Projection::new("space", "")
                            .with_walls(vec![wall])
                            .with_panels(panels),
                    ),
                }));
            if fetched {
                feed.conversation.apply(Event::Image {
                    hash: hash.clone(),
                    png: PNG.to_vec().into(),
                });
            }
            feed.conversation.take_actions();
            feed
        };
        let published: Vec<Panel> = (0..WALL_SLOTS)
            .map(|slot| {
                let place = Place::Wall {
                    wall: TOMORI_WALL.into(),
                    slot,
                };
                Panel::new(
                    format!("panel-{slot}"),
                    "Charts",
                    "**Published** on the wall.",
                    place,
                )
                .with_image(tomori_protocol::Image::new(
                    &hash,
                    1,
                    1,
                    PNG.len() as u32,
                ))
            })
            .collect();
        // The wall's face and each panel's are laid out, as a participant's frame step lays them out, pointed at by nobody.
        let mut feed = watching(published.clone(), true);
        let mut seen = frame(&mut feed, 1.0 / 60.0);
        for _ in 0..3 {
            seen = frame(&mut feed, 1.0 / 60.0);
        }
        assert!(
            seen.faces > WALL_SLOTS as usize,
            "the wall and its panels: {}",
            seen.faces
        );
        // With the image each image panel shows, for the renderer to draw over it.
        assert!(!seen.images.is_empty());
        assert!(
            seen.images
                .iter()
                .all(|(shown, png)| (shown.as_str(), &png[..]) == (hash.as_str(), &PNG[..]))
        );
        // And drawn, in an offscreen render of the spectator's view, over the wall without them.
        let draw = |gpu: &tomori_render::test_gpu::Gpu, feed: Feed| {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let mut renderer =
                crate::graphics::Renderer::new(&gpu.device, &gpu.queue, format, WIDTH, HEIGHT);
            let target = target(gpu, WIDTH, HEIGHT);
            let mut view = crate::spectator_view::SpectatorView::new(feed);
            for _ in 0..4 {
                render(gpu, &mut view, &mut renderer, &target);
            }
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectator panels readback"),
                size: u64::from(WIDTH * HEIGHT * 4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(WIDTH * 4),
                        rows_per_image: Some(HEIGHT),
                    },
                },
                target.size(),
            );
            gpu.queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let uploaded = renderer.images().levels(&hash).is_some();
            (
                buffer.slice(..).get_mapped_range().unwrap().to_vec(),
                uploaded,
            )
        };
        let changed = |a: &[u8], b: &[u8]| {
            a.chunks(4)
                .zip(b.chunks(4))
                .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 8))
                .count()
        };
        tomori_render::test_gpu::with(|gpu| {
            let (with_panels, uploaded) = draw(&gpu, watching(published.clone(), true));
            let (unfetched, unheld) = draw(&gpu, watching(published, false));
            let (without, _) = draw(&gpu, watching(Vec::new(), false));
            let drawn = changed(&unfetched, &without);
            assert!(
                drawn > 200,
                "the panels must draw over the wall: {drawn} pixels changed"
            );
            // The image is uploaded and drawn over its panels, where until it is fetched they show none.
            assert!(uploaded && !unheld);
            let image = changed(&with_panels, &unfetched);
            assert!(
                image > 20,
                "the image must draw over its panels: {image} pixels changed"
            );
        });
    }

    /// A copy frozen to a window, floating ahead, is drawn as a participant's client draws it: its chips, its time chip
    /// marked frozen, and its chart over that window. See `design/panel-context.md`.
    #[test]
    fn the_spectator_draws_a_frozen_copy_with_its_chips() {
        use tomori_protocol::chart::{AxisLabel, Chart, Colour, Series, SeriesStyle, TimeAxis};
        use tomori_protocol::context::{ChipPress, ContextChip, ContextSetting, ContextValue};
        use tomori_protocol::{Panel, Place, Projection};
        let mut feed = Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        );
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
        feed.conversation
            .apply(Event::Message(ServerMessage::Space {
                state: Box::new(SpaceState {
                    id: "space".into(),
                    projection: true,
                    ..Default::default()
                }),
            }));
        let (from_ms, to_ms) = (1_794_450_000_000, 1_794_451_800_000);
        let chart = Chart::new(10)
            .with_time(TimeAxis::Fixed { from_ms, to_ms })
            .with_labels(vec![
                AxisLabel::new(0, "11-12 02:20"),
                AxisLabel::new(9, "02:50 UTC"),
            ])
            .with_series(Series::new(
                "p99",
                Colour::Amber,
                SeriesStyle::Line,
                vec![Some(41.0); 10],
            ));
        let frozen = Panel::new(
            "panel-4",
            "Latency",
            chart.summary(),
            Place::Free {
                pose: tomori_protocol::Pose::upright_facing([0.0, 1.5, -1.6], [0.0, 1.6, 0.0]),
            },
        )
        .with_chart(chart)
        .with_context(vec![
            ContextChip::new(
                "region",
                "Region",
                ContextValue::Text {
                    text: "EU-WEST1".into(),
                },
                ContextSetting::Fixed,
            ),
            ContextChip::new(
                "time",
                "Time",
                ContextValue::Window { from_ms, to_ms },
                ContextSetting::Fixed,
            )
            .with_press(ChipPress::Unfreeze),
        ]);
        feed.conversation
            .apply(Event::Message(ServerMessage::Projection {
                projection: Box::new(Projection::new("space", "").with_panels(vec![frozen])),
            }));
        feed.conversation.take_actions();
        for _ in 0..3 {
            frame(&mut feed, 1.0 / 60.0);
        }
        let watched = feed.watch(&BTreeMap::new(), 4.0 / 3.0, 1.0 / 60.0, Instant::now());
        let face = watched
            .frame
            .projection
            .iter()
            .filter(|sheet| sheet.model.is_some())
            .filter_map(|sheet| sheet.canvas.as_deref())
            .map(|canvas| {
                canvas
                    .rows
                    .iter()
                    .map(tomori_scene::ui_text::Row::plain)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .find(|plain| plain.starts_with("Latency"))
            .expect("the frozen copy is drawn");
        assert!(face.contains("❄"), "{face}");
        assert!(face.contains("EU-WEST1"), "{face}");
    }

    #[test]
    fn with_nobody_to_follow_the_spectator_watches_from_its_spawn_point() {
        use tomori_protocol::spawn::{self, Spawn};
        let mut feed = connected(Sceneries::empty(), ConnectionRole::Owner);
        // The server is never asked where the spectator arrives, so it keeps no spawn point for it.
        assert!(!tomori_client::connection::Session::spawns(
            &feed.conversation
        ));
        let mut joined = |anchors: tomori_protocol::Anchors| {
            show(
                &mut feed,
                SpaceState {
                    id: "space".into(),
                    anchors,
                    ..Default::default()
                },
            );
            frame(&mut feed, 1.0 / 60.0).projection
        };
        // Its own point, by the gate 4 m along +X, facing +X, before the participants' at the campsite.
        let gate = tomori_protocol::Pose::upright([4.0, 0.0, 0.0], std::f32::consts::FRAC_PI_2);
        let camp = tomori_protocol::Pose::upright([0.0, 0.0, -6.0], 0.0);
        let projection = joined(
            tomori_protocol::Anchors::new()
                .with_spawn(Spawn::new(camp).with_tag(spawn::OWNER))
                .with_spawn(Spawn::new(gate).with_tag(spawn::SPECTATOR)),
        );
        let eye = tomori_render::scenery::eye(projection);
        assert!(eye.distance(Vec3::new(4.0, 1.6, 0.0)) < 1e-4, "{eye}");
        // Looking along +X, so that a point ahead of it is in the middle of the view.
        let ahead = projection.project_point3(eye + Vec3::X);
        assert!(ahead.x.abs() < 1e-4 && ahead.y.abs() < 1e-4, "{ahead}");
        // Without one of its own, the first untagged; with neither, home, as a participant joins.
        let projection = joined(
            tomori_protocol::Anchors::new()
                .with_spawn(Spawn::new(gate).with_tag(spawn::VISITOR))
                .with_spawn(Spawn::new(camp)),
        );
        let eye = tomori_render::scenery::eye(projection);
        assert!(eye.distance(Vec3::new(0.0, 1.6, -6.0)) < 1e-4, "{eye}");
        let projection = joined(
            tomori_protocol::Anchors::new().with_spawn(Spawn::new(gate).with_tag(spawn::VISITOR)),
        );
        assert!(tomori_render::scenery::eye(projection).distance(NEUTRAL) < 1e-4);
        assert_eq!(feed.neutral(), Mat4::from_translation(NEUTRAL));
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
        // Until it is given sceneries, it draws and lists the grid alone.
        let plain = Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        );
        assert_eq!(
            plain.conversation.environments(),
            Some(vec![tomori_protocol::EnvironmentVersion::new("minimal", 1)])
        );
        assert_eq!(plain.sceneries().environments(), [Environment::MINIMAL]);
    }
}
