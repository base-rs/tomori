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
    interaction, playback,
    spectator::Frame,
    swarm::{AGENT_COUNT, CENTER},
};
use glam::{Mat4, Quat, Vec3};
use protocol::{Hand, ServerMessage};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};

/// A participant's latest presence report, as relayed by the server.
#[derive(Clone)]
struct ParticipantPresence {
    head: protocol::Pose,
    left: Option<protocol::Pose>,
    right: Option<protocol::Pose>,
    hands: Option<[Option<Hand>; 2]>,
}

/// The options a spectator is started with.
#[derive(Clone, Debug, Default)]
pub struct SpectatorOptions {
    /// A server address to connect to straight away.
    pub server: Option<String>,
    /// A space id to join, as printed by `tomori discover`.
    pub space: Option<String>,
    /// A participant name to follow, or the first available.
    pub follow: Option<String>,
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
    sequence: u64,
    environment: Environment,
    last_poll: Instant,
    last_caption: Option<String>,
}

impl Feed {
    /// Connect to the server in `options.server`, join `options.space`, and
    /// follow `options.follow`.
    pub fn new(options: SpectatorOptions) -> Self {
        let overrides = Overrides {
            server: options.server.clone(),
            name: None,
        };
        let store = Store::new(None);
        let identity = Identity::new(&store, crate::identity::host_name().as_deref(), overrides);
        let mut conversation = Conversation::new(&identity.participant, &identity.name);

        // The connection enqueues a `Connect` action when a server is given,
        // so it must be created before `join_space` queues a `Send(Join)`:
        // the `Connect` is performed first, opening the client, and the
        // `Send(Join)` that follows then reaches the server.
        let mut connection = Connection::new(
            identity,
            store,
            Box::new(()),
            Box::new(capture::Desktop),
            Box::new(playback::Desktop),
            &mut conversation,
        );

        // If a space is given, join it directly, as the lobby's accept would.
        if let Some(space) = &options.space {
            conversation.join_space(space.clone());
        }

        // The presence hook writes each relayed presence into the shared map.
        let presence = Arc::new(Mutex::new(BTreeMap::new()));
        let hook = presence.clone();
        connection.watch_presence(move |message| {
            if let ServerMessage::Presence {
                source,
                head,
                left,
                right,
                hands,
            } = message
                && let Ok(mut map) = hook.lock()
            {
                map.insert(
                    *source,
                    ParticipantPresence {
                        head: *head,
                        left: *left,
                        right: *right,
                        hands: *hands,
                    },
                );
            }
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
            sequence: 0,
            environment: Environment::initial(),
            last_poll: Instant::now(),
            last_caption: None,
        }
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

        // Update the worker simulation from the space's state.
        let (root, workspace) = self.workers.update(self.conversation.joined_space());
        self.scene.update(root, workspace, dt);

        // Convert protocol poses to controller poses for the frame.
        let to_pose = |p: Option<protocol::Pose>| {
            p.map(|p| Pose {
                position: p.position,
                orientation: p.orientation,
            })
        };

        // A neutral spherical distribution, matching the swarm's own default,
        // since the spectator does not run the swarm simulation.
        let agents: Vec<[f32; 3]> = (0..AGENT_COUNT)
            .map(|i| {
                let y = 1.0 - 2.0 * (i as f32 + 0.5) / AGENT_COUNT as f32;
                let angle = i as f32 * 2.399_963_1;
                let r = (1.0 - y * y).sqrt();
                let offset = Vec3::new(r * angle.cos(), y * 0.6, r * angle.sin());
                (CENTER + offset * 0.85).to_array()
            })
            .collect();

        Frame {
            sequence: self.sequence,
            environment: self.environment,
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
            interaction: interaction::Visual::default(),
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
            Mode::Lobby => match following {
                Some(name) => format!("SPECTATOR\nWaiting for {name}..."),
                None => "SPECTATOR\nJoining space...".to_string(),
            },
            Mode::Space => match following {
                Some(name) => format!("SPECTATOR\nFollowing {name}"),
                None => "SPECTATOR\nIn space, waiting for a participant...".to_string(),
            },
        }
    }
}
