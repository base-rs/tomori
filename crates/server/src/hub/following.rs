// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent following whoever it is talking with, in a space whose owner chose **Follow the speaker**: it turns to
//! someone as they press X; after [`COMMIT`] of holding it goes to stand beside them, unless it already stands near
//! enough; while it answers someone else, it only glances at a new speaker, and goes to them as their words are given;
//! it stays [`DWELL`] before leaving one partner for another, serves a huddle from one spot, and goes home once the
//! conversation has been quiet for the space's delay. A pin, from **Call Tomori here** or `move_agent stay`, keeps it
//! where it is. Everything here is for this run only. See `design/environments.md`.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use tomori_protocol::{Activity, Movement, Pose};

use super::{Hub, HubState};
use crate::follow::{self, Route, Scene, Speaker};

/// How long X is held before the agent commits to going to the speaker: a tap, or a press let go at once, moves
/// nothing.
pub(crate) const COMMIT: Duration = Duration::from_millis(400);

/// How long the agent stays where it went for someone before it leaves for another partner.
pub(crate) const DWELL: Duration = Duration::from_secs(10);

/// How far, in metres, a new partner may stand from the agent and still wait for its [`DWELL`], if they see it.
pub(crate) const FAR: f32 = 6.0;

/// How lately someone spoke to it, to count as one of a huddle.
pub(crate) const LATELY: Duration = Duration::from_secs(60);

/// How long it glances at someone who began speaking while it answers another.
pub(crate) const GLANCE: Duration = Duration::from_secs(2);

/// How long the conversation stays quiet before it goes home once its partner has left or gone away.
pub(crate) const LEFT: Duration = Duration::from_secs(15);

/// How long a pin lasts with nobody in the space.
pub(crate) const EMPTY: Duration = Duration::from_secs(30 * 60);

/// How often, while it is away from home, whether to go home is checked.
pub(crate) const POLL: Duration = Duration::from_secs(1);

/// How fast it glides, in metres a second, at most, as the swarm's agents do.
pub(crate) const GLIDE: f32 = 0.85;

/// How fast it streams, in metres a second, at most, as a worker's flock flies.
pub(crate) const STREAMING: f32 = 6.0;

/// How long after its last frame of speech is sent the agent counts as still speaking: the frame's own lead and the
/// pause before held speech plays.
const SPOKEN: Duration = Duration::from_millis(700);

/// What a participant's press of X captured, as their words began: where they were and what they attended to, what the
/// agent keeps clear of, and where its home is.
#[derive(Clone, Debug)]
pub(crate) struct Captured {
    pub speaker: Speaker,
    pub scene: Scene,
    pub home: Pose,
}

/// A glide under way: from where, and where the agent was summoned to before, through which points, since when and how
/// fast.
#[derive(Clone, Debug)]
struct Glide {
    from: [f32; 3],
    was: Option<Pose>,
    path: Vec<[f32; 3]>,
    started: Instant,
    speed: f32,
}

impl Glide {
    /// How long it takes, end to end.
    fn length(&self) -> f32 {
        let mut at = self.from;
        self.path
            .iter()
            .map(|point| {
                let step = follow::distance(at, *point);
                at = *point;
                step
            })
            .sum()
    }

    /// When it ends.
    fn ends(&self) -> Instant {
        self.started + Duration::from_secs_f32(self.length() / self.speed)
    }

    /// Where it has reached at `now`.
    fn at(&self, now: Instant) -> [f32; 3] {
        let mut left = now.saturating_duration_since(self.started).as_secs_f32() * self.speed;
        let mut at = self.from;
        for point in &self.path {
            let step = follow::distance(at, *point);
            if left < step {
                let part = left / step.max(1e-6);
                return std::array::from_fn(|axis| at[axis] + (point[axis] - at[axis]) * part);
            }
            left -= step;
            at = *point;
        }
        at
    }
}

/// What a space keeps of its agent's following, for this run and environment only.
#[derive(Debug, Default)]
pub(crate) struct Following {
    /// The participant whose press of X waits to commit, and when it does.
    pending: Option<(String, Instant)>,
    /// When each participant's press of X under way counts as held, rather than a tap, whether or not it commits.
    holding: HashMap<String, Instant>,
    /// Those whose last press of X was a tap, let go before [`COMMIT`]: their words, if given, move nothing.
    tapped: Vec<String>,
    /// What each participant's last press of X captured.
    captured: HashMap<String, Captured>,
    /// The participant it talks with now, or last did.
    partner: Option<String>,
    /// When it last went somewhere for someone, and for whom.
    moved: Option<(Instant, String)>,
    /// Who spoke to it lately, and when.
    spoken: Vec<(String, Instant)>,
    /// Whether it is pinned where it stands.
    pub(super) pinned: bool,
    /// Since when the space has had nobody in it, while it is pinned.
    empty: Option<Instant>,
    /// Since when the conversation has been quiet, while it is away from home.
    quiet: Option<Instant>,
    /// Where it turned to face, while it does.
    facing: Option<[f32; 3]>,
    /// Where it glances, and until when.
    glance: Option<([f32; 3], Instant)>,
    glide: Option<Glide>,
    /// The waypoint its glide passes through, and whether it streams, as clients are told.
    pub(super) via: Option<[f32; 3]>,
    pub(super) stream: bool,
    /// When whether it goes home is next checked.
    poll: Option<Instant>,
    /// Until when the agent speaks, from the last frame of its speech sent.
    speaking: Option<Instant>,
}

impl Following {
    /// This glide begun `by` earlier, for tests of a move stopped on its way.
    #[cfg(test)]
    pub(super) fn backdate(&mut self, by: Duration) {
        self.glide.as_mut().expect("gliding").started -= by;
    }

    /// Where the agent turns to face, if anywhere: whomever it glances at, or whoever it turned to.
    pub(super) fn faces(&self) -> Option<[f32; 3]> {
        self.glance.map(|(eyes, _)| eyes).or(self.facing)
    }

    /// When something it does is next due, if anything is.
    pub(super) fn deadline(&self) -> Option<Instant> {
        [
            self.pending.as_ref().map(|(_, due)| *due),
            self.holding.values().min().copied(),
            self.glance.map(|(_, until)| until),
            self.glide.as_ref().map(Glide::ends),
            self.poll,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The agent's speech sent a frame lasting `duration` at `now`.
    pub(super) fn spoke(&mut self, now: Instant, duration: Duration) {
        let until = now + duration + SPOKEN;
        self.speaking = Some(self.speaking.map_or(until, |was| was.max(until)));
        // Its speech starting puts off its return, however short, between one check and the next.
        self.quiet = None;
    }

    /// Where the agent stands at `now`, summoned to `summoned` or else at `home`: along its glide, while it glides.
    fn at(&self, summoned: Option<Pose>, home: Pose, now: Instant) -> Pose {
        let pose = summoned.unwrap_or(home);
        match &self.glide {
            Some(glide) => Pose {
                position: glide.at(now),
                ..pose
            },
            None => pose,
        }
    }
}

/// Where the agent's home is among `anchors`: the environment's, or else the default.
pub(crate) fn home(anchors: &tomori_protocol::Anchors) -> Pose {
    anchors.agent.unwrap_or_else(|| {
        Pose::at(crate::stage::AGENT).facing_towards([0.0, crate::stage::AGENT[1], 0.0])
    })
}

impl Hub {
    /// What `participant`, in `space`, attends to now, as their client last reported, with what the agent keeps clear
    /// of there and where its home is: what they point at with their right controller, else what they look at, a wall,
    /// a panel or another person, else [`follow::GAZE`] along the way their head faces. None before they report where
    /// they are.
    pub(super) fn capture(
        &self,
        state: &HubState,
        space: &str,
        participant: &str,
    ) -> Option<Captured> {
        let seen = state
            .participants
            .get(participant)
            .expect("a participant is known")
            .seen
            .as_ref()?;
        let (scene, projection, home) = self.scene(state, space);
        let others: Vec<(&str, &str, [f32; 3])> = scene
            .heads
            .iter()
            .filter(|head| **head != seen.head.position)
            .map(|head| ("", "", *head))
            .collect();
        let point = |ray: &Pose| {
            let ahead = crate::stage::geometry::rotate(ray.orientation, [0.0, 0.0, -1.0]);
            crate::presence::meet(ray.position, ahead, &projection, &others).map(|(_, along)| {
                std::array::from_fn(|axis| ray.position[axis] + ahead[axis] * along)
            })
        };
        let focus = seen.controllers[1]
            .as_ref()
            .and_then(point)
            .or_else(|| point(&seen.head))
            .unwrap_or_else(|| follow::gaze(&seen.head));
        Some(Captured {
            speaker: Speaker {
                head: seen.head,
                focus,
            },
            scene,
            home,
        })
    }
}

impl Hub {
    /// Bring the agent of `space` to `participant`, and pin it there, for everyone, at `now`, as **Call Tomori here**
    /// does: beside them, as following stands it, or, with nowhere clear there, in front of them, as a summons stood it,
    /// its status wall and workers staying home. Refused where the space keeps it home, or before they report where
    /// they are.
    pub(super) fn call_agent(
        &self,
        state: &mut HubState,
        space: &str,
        participant: &str,
        now: Instant,
    ) -> Result<(), &'static str> {
        let movement = state
            .space(space)
            .map(|record| record.model.movement())
            .unwrap_or_default();
        if movement.mode == Movement::Home {
            return Err(
                "Tomori stays home in this space: an owner can let it move in the Space section.",
            );
        }
        let captured = self
            .capture(state, space, participant)
            .ok_or("Tomori does not know where you are yet.")?;
        let record = state.space(space).expect("found above");
        let from = record
            .following
            .at(record.summoned, captured.home, now)
            .position;
        let head = captured.speaker.head;
        let at = follow::spot(&captured.speaker, &captured.scene)
            .unwrap_or_else(|| crate::presence::before(&head));
        let route = follow::route(from, at.position, &captured.scene);
        let was = record.summoned;
        state.summon(space, Some(at), Some(head.position));
        state.agent_moved(space, (from, was), at, route, now);
        state.agent_pin(space, now);
        let name = &state.participants[participant].name;
        log::info!("{name} called the agent of space {space} to them, where it stays.");
        Ok(())
    }

    /// Move the agent of `space` to `at`, before someone whose eyes are at `eyes`, as its own `move_agent` asked, at
    /// `now`, its status wall and workers staying home. Whether it moved.
    pub(super) fn agent_goes(
        &self,
        state: &mut HubState,
        space: &str,
        at: Pose,
        eyes: [f32; 3],
        now: Instant,
    ) -> bool {
        let record = state.space(space).expect("the caller found it");
        let (scene, _, home) = self.scene(state, space);
        let from = record.following.at(record.summoned, home, now).position;
        let route = follow::route(from, at.position, &scene);
        let was = record.summoned;
        let moved = state.summon(space, Some(at), Some(eyes));
        if moved {
            state.agent_moved(space, (from, was), at, route, now);
        }
        moved
    }

    /// What the agent of `space` keeps clear of now, the environment's solids, the walls shown, the panels floating and
    /// everyone's heads; what the space projects; and where the agent's home is.
    fn scene(&self, state: &HubState, space: &str) -> (Scene, tomori_protocol::Projection, Pose) {
        let record = state.space(space).expect("the caller found it");
        let environments = self.environments();
        let environment = &record.model.environment;
        let walls = environments.walls(environment);
        let projection = record.stage.view(space, &walls, crate::cards::unix_now());
        let panels = projection
            .panels
            .iter()
            .filter_map(|panel| match &panel.place {
                tomori_protocol::Place::Free { pose } => Some(*pose),
                _ => None,
            })
            .collect();
        let heads = record
            .model
            .participants
            .iter()
            .filter_map(|key| state.participants.get(key))
            .filter_map(|participant| participant.seen.as_ref())
            .map(|seen| seen.head.position)
            .collect();
        let scene = Scene::new(
            &environments.solids(environment),
            &projection.walls,
            panels,
            heads,
        );
        (scene, projection, home(&environments.anchors(environment)))
    }
}

impl HubState {
    /// How the agent of `space` moves changed, as `name` set it: kept home, it goes home now; otherwise it stays where it
    /// is, and follows from the next words if it now follows the speaker.
    pub(super) fn movement_changed(&mut self, space: &str, name: &str, now: Instant) {
        let record = self.space_mut(space).expect("the caller found it");
        if record.model.movement().mode == Movement::Home {
            self.agent_home(space, &format!("{name} keeps it home"));
            return;
        }
        let following = &mut record.following;
        following.pending = None;
        following.glance = None;
        following.quiet = None;
        following.poll = Some(now + POLL);
        self.woken.notify(usize::MAX);
    }

    /// `participant`, in `space`, began words for its agent at `now`, as `captured` found them: in a space whose agent
    /// follows the speaker, it turns to them at once and, if it is free, goes to them once they have held X for
    /// [`COMMIT`]; while it answers someone else, or is pinned, it glances at them only. Pinned where it moves only
    /// when asked, it still turns to them, as a pinned agent does wherever it follows; unpinned there, or kept home, it
    /// neither turns nor moves. Visitors, whose words go nowhere, and words that answer, name or title, never come here.
    pub(super) fn agent_pressed(
        &mut self,
        space: &str,
        participant: &str,
        captured: Captured,
        now: Instant,
    ) {
        let others_speaking = self
            .utterances
            .values()
            .any(|record| record.space == space && record.participant != participant);
        let record = self
            .space_mut(space)
            .expect("words begin in a listed space");
        let mode = record.model.movement().mode;
        if mode != Movement::Follow && !(mode == Movement::Asked && record.following.pinned) {
            return;
        }
        let busy = others_speaking
            || matches!(
                record.model.activity(),
                Activity::Running | Activity::Waiting
            )
            || record.model.waiting() > 0;
        let following = &mut record.following;
        let eyes = captured.speaker.head.position;
        following
            .spoken
            .retain(|(who, at)| who != participant && now.saturating_duration_since(*at) < LATELY);
        following.spoken.push((participant.to_owned(), now));
        following.captured.insert(participant.to_owned(), captured);
        following.tapped.retain(|who| who != participant);
        following
            .holding
            .insert(participant.to_owned(), now + COMMIT);
        following.quiet = None;
        if busy || following.pinned {
            following.glance = Some((eyes, now + GLANCE));
            if following.pinned && !busy {
                following.facing = Some(eyes);
            }
        } else {
            following.facing = Some(eyes);
            following.glance = None;
            following.pending = Some((participant.to_owned(), now + COMMIT));
        }
        following.poll.get_or_insert(now + POLL);
        self.touch_space(space);
        self.woken.notify(usize::MAX);
    }

    /// `participant` let go of X at `now`: a press let go before [`COMMIT`] was a tap, and moves nothing, neither now
    /// nor as its words are given.
    pub(super) fn agent_released(&mut self, space: &str, participant: &str, now: Instant) {
        if let Some(record) = self.space_mut(space)
            && record
                .following
                .holding
                .remove(participant)
                .is_some_and(|held| held > now)
        {
            let following = &mut record.following;
            following.tapped.push(participant.to_owned());
            following.pending = following
                .pending
                .take()
                .filter(|(who, _)| who != participant);
        }
    }

    /// `participant`'s words came to nothing for the agent, cancelled, failed or with no speech in them, at `now`: a
    /// press waiting to commit moves nothing, and a move made for them stops where it has reached.
    pub(super) fn agent_cancelled(&mut self, space: &str, participant: &str, now: Instant) {
        let Some(record) = self.space_mut(space) else {
            return;
        };
        let following = &mut record.following;
        following.holding.remove(participant);
        if following
            .pending
            .as_ref()
            .is_some_and(|(who, _)| who == participant)
        {
            following.pending = None;
        }
        let gliding = following
            .moved
            .as_ref()
            .is_some_and(|(_, whom)| whom == participant)
            && following
                .glide
                .as_ref()
                .is_some_and(|glide| glide.ends() > now);
        if !gliding {
            return;
        }
        let glide = following.glide.take().expect("checked above");
        following.via = None;
        following.stream = false;
        following.quiet = None;
        let reached = glide.at(now);
        // Barely begun, it stays where it was.
        record.summoned = if follow::distance(reached, glide.from) < 0.05 {
            glide.was
        } else {
            record.summoned.map(|summoned| Pose {
                position: reached,
                ..summoned
            })
        };
        if record.summoned.is_none() {
            record.partner = None;
        }
        log::info!("The agent of space {space} stopped where it was, as nothing was said.");
        self.touch_space(space);
    }

    /// The agent of `space` was given the words of the participant numbered `source` at `now`: in a space whose agent
    /// follows the speaker, it goes to them as it begins answering them, unless their press was a tap.
    pub(super) fn agent_given(&mut self, space: &str, source: u32, now: Instant) {
        let record = self.space(space).expect("given in a listed space");
        // Unless they have left meanwhile.
        if let Some(participant) = record
            .model
            .participants
            .iter()
            .find(|key| {
                self.participants
                    .get(*key)
                    .is_some_and(|participant| participant.source == source)
            })
            .cloned()
        {
            let following = &mut self.space_mut(space).expect("found above").following;
            let tapped = following.tapped.contains(&participant);
            following.tapped.retain(|who| *who != participant);
            if !tapped {
                self.follow(space, &participant, now, "it began answering them");
            }
        }
    }

    /// Go to `participant` in `space` at `now`, for `reason`, as following the speaker does, from what their last press
    /// captured; or only turn to them, where it already stands near enough, stays its [`DWELL`] for another, or finds no
    /// spot clear. Nothing while it is pinned, or in a space whose agent does not follow.
    fn follow(&mut self, space: &str, participant: &str, now: Instant, reason: &str) {
        let name = self
            .participants
            .get(participant)
            .map(|record| record.name.clone())
            .unwrap_or_default();
        let record = self.space_mut(space).expect("the caller found it");
        if record.model.movement().mode != Movement::Follow || record.following.pinned {
            return;
        }
        let members = record.model.participants.clone();
        let following = &mut record.following;
        let Some(captured) = following.captured.get(participant).cloned() else {
            return;
        };
        let agent = following.at(record.summoned, captured.home, now);
        let speaker = captured.speaker;
        let eyes = speaker.head.position;
        let scene = &captured.scene;
        following.partner = Some(participant.to_owned());
        let turn = |following: &mut Following| {
            following.facing = Some(eyes);
            following.poll.get_or_insert(now + POLL);
        };
        if follow::near_enough(&agent, &speaker, scene) {
            turn(following);
            self.touch_space(space);
            return;
        }
        // It does not hop from one partner to the next, unless the new one is far or cannot see it.
        let dwelling = following.moved.as_ref().is_some_and(|(at, whom)| {
            whom != participant && now.saturating_duration_since(*at) < DWELL
        }) && follow::distance(agent.position, eyes) <= FAR
            && follow::sees(&agent, eyes, scene);
        if dwelling {
            turn(following);
            self.touch_space(space);
            return;
        }
        // Several who spoke to it lately, standing together, are served from one spot.
        let mut group = vec![speaker];
        group.extend(
            following
                .spoken
                .iter()
                .filter(|(who, at)| {
                    who != participant
                        && now.saturating_duration_since(*at) < LATELY
                        && members.contains(who)
                })
                .filter_map(|(who, _)| following.captured.get(who))
                .map(|captured| captured.speaker)
                .filter(|other| follow::together(other.head.position, eyes)),
        );
        let huddle = group.len() > 1;
        let spot = if huddle {
            follow::huddle(&group, scene)
        } else {
            follow::spot(&speaker, scene)
        };
        let Some(spot) = spot else {
            turn(following);
            self.touch_space(space);
            return;
        };
        let route = follow::route(agent.position, spot.position, scene);
        following.start(agent.position, record.summoned, spot.position, route, now);
        following.facing = None;
        following.glance = None;
        following.quiet = None;
        // Its dwell runs from when it arrives.
        let arrives = following.glide.as_ref().map_or(now, Glide::ends);
        following.moved = Some((arrives, participant.to_owned()));
        following.poll.get_or_insert(now + POLL);
        record.summoned = Some(spot);
        record.partner = Some(eyes);
        let with = if huddle {
            format!("{name} and those with them")
        } else {
            name
        };
        log::info!("The agent of space {space} followed {with}, as {reason}.");
        self.touch_space(space);
        self.woken.notify(usize::MAX);
    }

    /// Send the agent of `space` home, unpinned and facing as its home faces, for `reason`,
    /// logged if it moved; whether it moved.
    pub(super) fn agent_home(&mut self, space: &str, reason: &str) -> bool {
        let record = self.space_mut(space).expect("the caller found it");
        let turned = record.following.faces().is_some() || record.following.pinned;
        record.following = Following {
            captured: std::mem::take(&mut record.following.captured),
            spoken: std::mem::take(&mut record.following.spoken),
            holding: std::mem::take(&mut record.following.holding),
            tapped: std::mem::take(&mut record.following.tapped),
            speaking: record.following.speaking,
            ..Following::default()
        };
        let moved = self.summon(space, None, None);
        if moved {
            log::info!("The agent of space {space} went home, as {reason}.");
        } else if turned {
            self.touch_space(space);
        }
        moved
    }

    /// Pin the agent of `space` where it is going, or stands, so that it follows nobody and does not go home on its own.
    pub(super) fn agent_pin(&mut self, space: &str, now: Instant) {
        let following = &mut self
            .space_mut(space)
            .expect("the caller found it")
            .following;
        following.pinned = true;
        following.pending = None;
        following.empty = None;
        following.poll.get_or_insert(now + POLL);
        self.touch_space(space);
        self.woken.notify(usize::MAX);
    }

    /// The agent of `space` was moved by a participant or by its own tool to `at`, from `from`, where it was summoned to
    /// `was`, at `now`, by `route`: its glide is followed from there, so that where it is stays known, and the return
    /// home waits for a quiet conversation again.
    pub(super) fn agent_moved(
        &mut self,
        space: &str,
        (from, was): ([f32; 3], Option<Pose>),
        at: Pose,
        route: Route,
        now: Instant,
    ) {
        let following = &mut self
            .space_mut(space)
            .expect("the caller found it")
            .following;
        following.start(from, was, at.position, route, now);
        following.facing = None;
        following.glance = None;
        following.quiet = None;
        following.pending = None;
        following.poll.get_or_insert(now + POLL);
        self.woken.notify(usize::MAX);
    }

    /// Whether the conversation in `space` is quiet at `now`: nobody records or is transcribed for it, nothing is queued,
    /// no approval or question waits, its agent's session is idle, and no agent speech plays or is held for anyone.
    fn conversation_quiet(&self, space: &str, now: Instant) -> bool {
        let record = self.space(space).expect("the caller found it");
        !self
            .utterances
            .values()
            .any(|utterance| utterance.space == space)
            && record.model.waiting() == 0
            && matches!(record.model.activity(), Activity::Idle | Activity::Unknown)
            && record.speech.quiet()
            && record.following.speaking.is_none_or(|until| until <= now)
    }

    /// When the agent of any space next does something, if any does: commits to a press, stops glancing, arrives or
    /// checks whether to go home.
    pub(super) fn movement_deadline(&self) -> Option<Instant> {
        self.spaces
            .iter()
            .filter_map(|space| space.following.deadline())
            .min()
    }

    /// Do what the agents of the spaces do by `now`: commit to presses held long enough, stop glancing, arrive, and go
    /// home once a conversation has been quiet for its space's delay, or a pin has stood with nobody in the space for
    /// [`EMPTY`].
    pub(super) fn movements_due(&mut self, now: Instant) {
        let ids: Vec<String> = self
            .spaces
            .iter()
            .filter(|space| space.following.deadline().is_some_and(|due| due <= now))
            .map(|space| space.model.id.clone())
            .collect();
        for id in ids {
            self.movement_due(&id, now);
        }
    }

    fn movement_due(&mut self, space: &str, now: Instant) {
        let quiet = self.conversation_quiet(space, now);
        let partner_gone = |state: &HubState, partner: &Option<String>| {
            partner.as_ref().is_some_and(|partner| {
                !state
                    .space(space)
                    .is_some_and(|record| record.model.participants.contains(partner))
                    || state.participants.get(partner).is_some_and(|record| {
                        record.engagement == Some(tomori_protocol::Engagement::Away)
                    })
            })
        };
        let gone = self
            .space(space)
            .is_some_and(|record| partner_gone(self, &record.following.partner));
        let record = self.space_mut(space).expect("found above");
        let setting = record.model.movement();
        let empty = record.model.participants.is_empty();
        let away = record.summoned.is_some();
        let following = &mut record.following;
        let mut changed = false;
        // A press held long enough commits.
        let committed = match &following.pending {
            Some((_, due)) if *due <= now => following.pending.take().map(|(who, _)| who),
            _ => None,
        };
        // A press held this long is no tap, whenever it is let go.
        following.holding.retain(|_, held| *held > now);
        if following.glance.is_some_and(|(_, until)| until <= now) {
            following.glance = None;
            changed = true;
        }
        if following
            .glide
            .as_ref()
            .is_some_and(|glide| glide.ends() <= now)
        {
            following.glide = None;
            // Arrived, it no longer goes by its waypoint or streams, for whoever joins.
            changed |= following.via.take().is_some() | std::mem::take(&mut following.stream);
        }
        let mut home = None;
        if following.poll.is_some_and(|due| due <= now) {
            following.poll = None;
            if following.pinned {
                match (empty, following.empty) {
                    (false, _) => following.empty = None,
                    (true, None) => following.empty = Some(now),
                    (true, Some(since)) if now.saturating_duration_since(since) >= EMPTY => {
                        home = Some("nobody had been in the space for half an hour");
                    }
                    (true, Some(_)) => {}
                }
                following.poll = Some(now + POLL);
            } else if setting.mode == Movement::Follow && (away || following.facing.is_some()) {
                if quiet {
                    let since = *following.quiet.get_or_insert(now);
                    let delay = setting.return_after.map(|delay| {
                        let delay = Duration::from_secs(u64::from(delay));
                        if gone { delay.min(LEFT) } else { delay }
                    });
                    if delay.is_some_and(|delay| now.saturating_duration_since(since) >= delay) {
                        home = Some("the conversation had been quiet");
                    }
                } else {
                    following.quiet = None;
                }
                following.poll = Some(now + POLL);
            }
        }
        if let Some(reason) = home {
            self.agent_home(space, reason);
        } else if changed {
            self.touch_space(space);
        }
        if let Some(participant) = committed {
            self.follow(space, &participant, now, "they began speaking to it");
        }
    }
}

impl Following {
    /// Begin a glide from `from`, where it was summoned to `was`, to `to` by `route` at `now`, as clients are told.
    fn start(
        &mut self,
        from: [f32; 3],
        was: Option<Pose>,
        to: [f32; 3],
        route: Route,
        now: Instant,
    ) {
        let (path, speed) = match route {
            Route::Straight => (vec![to], GLIDE),
            Route::Via(via) => (vec![via, to], GLIDE),
            Route::Stream => (vec![to], STREAMING),
        };
        self.via = match route {
            Route::Via(via) => Some(via),
            _ => None,
        };
        self.stream = route == Route::Stream;
        self.glide = Some(Glide {
            from,
            was,
            path,
            started: now,
            speed,
        });
    }
}

#[cfg(test)]
mod tests;
