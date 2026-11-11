// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent following whoever it is talking with, in a space whose owner chose **Follow the speaker**: it turns to
//! someone as they press X; after [`COMMIT`] of holding it comes straight towards them, unless it already stands near
//! enough; while it answers someone else, it only glances at a new speaker, and goes to them as their words are given;
//! it stays [`DWELL`] before leaving one partner for another, serves a huddle from one spot, and goes home once the
//! conversation has been quiet for the space's delay. Through its partner's exchange with it, as they speak, while it
//! thinks and while it replies, it keeps with them: it turns to them as they move and, once they settle more than
//! [`follow::NEAR`] away, follows them on. A pin, from **Call Tomori here** or `move_agent stay`, keeps it where it is.
//! Everything here is for this run only. See `design/environments.md`.
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

/// How fast it glides, in metres a second, at most, as clients glide it (`tomori-scene`'s `glide::SPEED`).
pub(crate) const GLIDE: f32 = 1.0625;

/// How fast it streams, in metres a second, at most, as a worker's flock flies.
pub(crate) const STREAMING: f32 = 6.0;

/// How long after its last frame of speech is sent the agent counts as still speaking: the frame's own lead and the
/// pause before held speech plays.
const SPOKEN: Duration = Duration::from_millis(700);

/// How long speech still to come counts towards the agent's reply with nothing of it queued, sent or finished meanwhile,
/// as when synthesis hangs: its reply then ends, so that it never keeps with someone, nor stays away from home, for ever.
pub(crate) const STALLED: Duration = Duration::from_secs(60);

/// How long whoever the agent keeps with, through their exchange with it, stands still before it follows them on, if
/// they have gone beyond where it serves them: it waits for them to settle rather than chasing each step, and never
/// moves while they travel, as by a teleport, only once they arrive.
pub(crate) const SETTLE: Duration = Duration::from_millis(750);

/// How long whoever the agent keeps with may go on moving or looking about, never still for [`SETTLE`], before it
/// checks anyway whether to follow them on, so that it is never left behind for long.
pub(crate) const RESTLESS: Duration = Duration::from_secs(3);

/// How far, in metres, a head may move and still stand still, for [`SETTLE`].
pub(crate) const STILL: f32 = 0.25;

/// How far, in degrees, a head may turn about the vertical and still stand still, for [`SETTLE`].
pub(crate) const STILL_TURN: f32 = 15.0;

/// How far, in degrees, whoever it keeps with may stand from the way it faces before it turns to them again, so that
/// it follows them round smoothly without turning at each sway of their head.
pub(crate) const FACE: f32 = 5.0;

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

/// Whom the agent keeps with through their exchange with it, as they speak to it, while it thinks and while it replies:
/// where their head last stood still, the way it faced there, since when they have moved since it last checked, and
/// when it checks whether it follows them on: once they have stood still for [`SETTLE`], or [`RESTLESS`] after they
/// began moving.
#[derive(Clone, Debug)]
struct Keeping {
    who: String,
    at: [f32; 3],
    heading: f32,
    since: Option<Instant>,
    due: Option<Instant>,
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
    pub(super) partner: Option<String>,
    /// When it last went somewhere for someone, and for whom.
    moved: Option<(Instant, String)>,
    /// Who spoke to it lately, and when.
    spoken: Vec<(String, Instant)>,
    /// Whether it is pinned where it stands.
    pub(super) pinned: bool,
    /// Who pinned it, by their handle, where a participant did, with **Call Tomori here** or **Tomori: stay here**, rather than
    /// the agent itself: only someone's words free a participant's pin for the agent's own `follow`.
    pub(super) pinned_by: Option<String>,
    /// Where it was, and when, as it was sent home: it glides home from there.
    homing: Option<([f32; 3], Instant)>,
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
    /// How many segments of its speech are queued for synthesis, or synthesized and not yet all sent: speech still to
    /// come, after its turn may have ended, which its reply lasts through as much as speech playing.
    unspoken: usize,
    /// When its speech last made progress: a segment queued, a frame sent, or a segment sent whole.
    progressed: Option<Instant>,
    /// The participant whose words it was last given, until its reply to them is over: its turn ended and its speech
    /// heard.
    answering: Option<String>,
    /// Whom it keeps with, while it does.
    keeping: Option<Keeping>,
}

impl Following {
    /// This glide begun `by` earlier, for tests of a move stopped on its way.
    #[cfg(test)]
    pub(super) fn backdate(&mut self, by: Duration) {
        self.glide.as_mut().expect("gliding").started -= by;
    }

    /// Whoever it keeps with began moving `by` earlier, for tests of someone who never stands still.
    #[cfg(test)]
    pub(super) fn restless(&mut self, by: Duration) {
        let keeping = self.keeping.as_mut().expect("kept with");
        keeping.since = keeping.since.map(|since| since - by);
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
            self.keeping.as_ref().and_then(|keeping| keeping.due),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The agent's speech sent a frame lasting `duration` at `now`.
    pub(super) fn spoke(&mut self, now: Instant, duration: Duration) {
        let until = now + duration + SPOKEN;
        self.speaking = Some(self.speaking.map_or(until, |was| was.max(until)));
        self.progressed = Some(now);
        // Its speech starting puts off its return, however short, between one check and the next.
        self.quiet = None;
    }

    /// `segments` more segments of the agent's speech, at least one, were queued for synthesis: its reply goes on until
    /// they are sent, however long synthesis takes after its turn has ended, and its return is put off.
    pub(super) fn queued(&mut self, segments: usize, now: Instant) {
        self.unspoken += segments;
        self.progressed = Some(now);
        self.quiet = None;
    }

    /// One segment queued for its speech has been sent whole, or could not be synthesized.
    pub(super) fn sent(&mut self, now: Instant) {
        self.unspoken = self.unspoken.saturating_sub(1);
        self.progressed = Some(now);
    }

    /// Synthesis stopped: nothing more queued for its speech will be sent.
    pub(super) fn unqueued(&mut self) {
        self.unspoken = 0;
    }

    /// Whether any of its speech is still to come at `now`: queued for synthesis, or synthesized and not yet all sent,
    /// unless it has made no progress for [`STALLED`].
    pub(super) fn unspoken(&self, now: Instant) -> bool {
        self.unspoken > 0
            && self
                .progressed
                .is_some_and(|at| now.saturating_duration_since(at) < STALLED)
    }

    /// Where the agent stands at `now`, summoned to `summoned` or else at `home`: along its glide, while it glides,
    /// or its way home, while it goes home.
    pub(super) fn at(&self, summoned: Option<Pose>, home: Pose, now: Instant) -> Pose {
        let pose = summoned.unwrap_or(home);
        match self.glide.clone().or_else(|| self.homeward(summoned, home)) {
            Some(glide) => Pose {
                position: glide.at(now),
                ..pose
            },
            None => pose,
        }
    }

    /// Its way home, at home's `home`, while it was sent home and is summoned nowhere since.
    fn homeward(&self, summoned: Option<Pose>, home: Pose) -> Option<Glide> {
        let (from, started) = self.homing.filter(|_| summoned.is_none())?;
        Some(Glide {
            from,
            was: None,
            path: vec![home.position],
            started,
            speed: GLIDE,
        })
    }

    /// Whether it is on its way somewhere at `now`, or home.
    pub(super) fn moving(&self, summoned: Option<Pose>, home: Pose, now: Instant) -> bool {
        self.glide
            .clone()
            .or_else(|| self.homeward(summoned, home))
            .is_some_and(|glide| glide.ends() > now)
    }

    /// How long, at `now`, until the agent goes home if nothing more is said, where the space's `movement` has it
    /// follow the speaker and return, and it is away and not pinned: the return delay, or [`LEFT`] where `gone`, its
    /// partner having left or gone away, less how long the conversation has been quiet.
    pub(super) fn returns_in(
        &self,
        away: bool,
        movement: tomori_protocol::AgentMovement,
        gone: bool,
        now: Instant,
    ) -> Option<Duration> {
        if self.pinned || !away || movement.mode != Movement::Follow {
            return None;
        }
        let delay = Duration::from_secs(u64::from(movement.return_after?));
        let delay = if gone { delay.min(LEFT) } else { delay };
        let quiet = self
            .quiet
            .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        Some(delay.saturating_sub(quiet))
    }

    /// The participant it talks with now, or last did, by their key.
    pub(super) fn partner(&self) -> Option<&str> {
        self.partner.as_deref()
    }
}

impl super::SpaceRecord {
    /// What a panel floating beside someone keeps clear of in this space, among `environments`: its environment's
    /// solids, and the agent where it stands or goes, so that, wherever the agent came to them from, the panel goes to
    /// the side of them it is not on. See `design/projection.md`.
    pub(super) fn panel_solids(
        &self,
        environments: &crate::environments::Environments,
    ) -> Vec<tomori_environment::sightlines::Solid> {
        let environment = &self.model.environment;
        let mut solids = environments.solids(environment);
        let agent = self
            .summoned
            .unwrap_or_else(|| home(&environments.anchors(environment)));
        solids.push(follow::body(&agent));
        solids
    }
}

/// Where the agent's home is among `anchors`: the environment's, or else the default.
pub(crate) fn home(anchors: &tomori_protocol::Anchors) -> Pose {
    anchors.agent.unwrap_or_else(|| {
        Pose::at(crate::stage::AGENT).facing_towards([0.0, crate::stage::AGENT[1], 0.0])
    })
}

impl Hub {
    /// Where `participant`, in `space`, stands now, as their client last reported, with what the agent keeps clear of
    /// there and where its home is. None before they report where they are.
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
        let (scene, _, home) = self.scene(state, space);
        Some(Captured {
            speaker: Speaker { head: seen.head },
            scene,
            home,
        })
    }
}

impl Hub {
    /// What the `presence` and `move_agent` tools say of the agent of `space` at `now`, among `environments`: where it
    /// stands, its mode, whom it talks with, who pinned it and when it goes home. See `design/mcp.md`.
    pub(super) fn agent_report(
        state: &HubState,
        environments: &crate::environments::Environments,
        space: &str,
        now: Instant,
    ) -> serde_json::Value {
        let record = state.space(space).expect("the caller found it");
        let anchors = environments.anchors(&record.model.environment);
        let following = &record.following;
        let home = home(&anchors);
        let moving = following.moving(record.summoned, home, now);
        // Where it has reached, while it is on its way.
        let partner = following
            .partner()
            .and_then(|key| state.participants.get(key))
            .map(|partner| crate::presence::handle(&partner.name, partner.tag));
        let report = crate::presence::AgentState {
            summoned: record.summoned,
            at: moving.then(|| following.at(record.summoned, home, now)),
            facing: following.faces(),
            pinned: following.pinned,
            pinned_by: following.pinned_by.clone(),
            partner,
            returns_in: following.returns_in(
                record.summoned.is_some(),
                record.model.movement(),
                state.partner_gone(space),
                now,
            ),
            movement: record.model.movement(),
            anchor: record.agent_anchor().map(str::to_owned),
        };
        crate::presence::agent(&anchors, &report)
    }

    /// `participant`, in `space`, reported where they are at `now`: if the agent keeps with them, it turns to them
    /// once they stand more than [`FACE`] from the way it faces, unless it is on its way, and, once they move beyond
    /// [`STILL`] or turn beyond [`STILL_TURN`], it waits for them to stand still for [`SETTLE`], or [`RESTLESS`] at most,
    /// before checking whether to follow them on. It keeps with nobody once their exchange is over. See `design/environments.md`.
    pub(super) fn agent_seen(
        &self,
        state: &mut HubState,
        space: &str,
        participant: &str,
        now: Instant,
    ) {
        let keeps = state.keeps_with(space, now);
        let head = state
            .participants
            .get(participant)
            .and_then(|record| record.seen.as_ref())
            .map(|seen| seen.head);
        if keeps.is_none() {
            state.keep_with_nobody(space);
            return;
        }
        let woken = state.woken.clone();
        let record = state.space_mut(space).expect("the caller found it");
        let Some(head) = head.filter(|_| keeps.as_deref() == Some(participant)) else {
            return;
        };
        let home = home(&self.environments().anchors(&record.model.environment));
        let following = &mut record.following;
        let keeping = following.keeping.as_mut().expect("kept with");
        let heading = follow::heading(&head);
        if follow::distance(head.position, keeping.at) > STILL
            || follow::apart(heading, keeping.heading) > STILL_TURN.to_radians()
        {
            keeping.at = head.position;
            keeping.heading = heading;
            let since = *keeping.since.get_or_insert(now);
            keeping.due = Some((now + SETTLE).min(since + RESTLESS));
            woken.notify(usize::MAX);
        }
        if following.moving(record.summoned, home, now) {
            return;
        }
        // It keeps facing them, as they move about, without turning at each sway.
        let agent = following.at(record.summoned, home, now);
        let faces = following.facing.map_or_else(
            || follow::faces(&agent),
            |facing| follow::towards(agent.position, facing),
        );
        if follow::apart(follow::towards(agent.position, head.position), faces) <= FACE.to_radians()
        {
            return;
        }
        following.facing = Some(head.position);
        following.poll.get_or_insert(now + POLL);
        if record.summoned.is_some() {
            record.partner = Some(head.position);
        }
        if state.touch_space_paced(space, now) {
            woken.notify(usize::MAX);
        }
    }

    /// Follow on whoever the agents of the spaces keep with, where they have stood still for [`SETTLE`] by `now` since
    /// they moved, or kept moving for [`RESTLESS`]: where the agent no longer serves them, by the rules it went to them by, it goes to them again, or to
    /// the huddle they stand in, but stays where it serves a huddle they are still one of, and waits its [`DWELL`] for
    /// another partner as ever.
    pub(super) fn keep_up(&self, state: &mut HubState, now: Instant) {
        let due: Vec<String> = state
            .spaces
            .iter()
            .filter(|space| {
                space
                    .following
                    .keeping
                    .as_ref()
                    .and_then(|keeping| keeping.due)
                    .is_some_and(|due| due <= now)
            })
            .map(|space| space.model.id.clone())
            .collect();
        for space in due {
            let keeps = state.keeps_with(&space, now);
            let record = state.space_mut(&space).expect("listed above");
            let keeping = record.following.keeping.as_mut().expect("listed above");
            keeping.due = None;
            keeping.since = None;
            let Some(participant) = keeps else {
                state.keep_with_nobody(&space);
                continue;
            };
            self.keep_up_with(state, &space, &participant, now);
        }
    }

    /// Follow on `participant`, whom the agent of `space` keeps with, as [`keep_up`](Self::keep_up) does, to where they
    /// stand now.
    fn keep_up_with(&self, state: &mut HubState, space: &str, participant: &str, now: Instant) {
        // They are kept with only once they have reported where they are.
        if let Some(captured) = self.capture(state, space, participant) {
            self.keep_up_from(state, space, participant, &captured, now);
        }
    }

    /// Follow on `participant`, as [`keep_up_with`](Self::keep_up_with) does, from where `captured` finds them now.
    fn keep_up_from(
        &self,
        state: &mut HubState,
        space: &str,
        participant: &str,
        captured: &Captured,
        now: Instant,
    ) {
        let eyes = captured.speaker.head.position;
        let record = state.space(space).expect("the caller found it");
        let following = &record.following;
        let others: Vec<Speaker> = following
            .spoken
            .iter()
            .filter(|(who, at)| {
                who != participant
                    && now.saturating_duration_since(*at) < LATELY
                    && record.model.participants.contains(who)
            })
            .filter_map(|(who, _)| self.capture(state, space, who))
            .map(|captured| captured.speaker)
            .filter(|other| follow::together(other.head.position, eyes))
            .collect();
        let agent = following.at(record.summoned, captured.home, now);
        if !others.is_empty() {
            let mut huddle = vec![captured.speaker];
            huddle.extend(others.iter().copied());
            if follow::serves(&agent, &huddle) {
                return;
            }
        }
        let reason = if following.answering.as_deref() == Some(participant) {
            "they moved while it answered them"
        } else {
            "they moved as they spoke to it"
        };
        state.go_to(space, participant, captured, others, now, reason);
    }

    /// Bring the agent of `space` to `participant`, and pin it there, for everyone, at `now`, as **Call Tomori here**
    /// does: at their conversation spot, as following stands it, or, with nowhere clear there, in front of them, as a
    /// summons stood it, its status wall and workers staying home. Refused where the space keeps it home, or before they
    /// report where they are.
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
        let at = follow::spot(from, &captured.speaker, &captured.scene)
            .unwrap_or_else(|| crate::presence::before(&head));
        let route = follow::route(from, at.position, &captured.scene);
        let was = record.summoned;
        state.summon(space, Some(at), Some(head.position));
        state.agent_moved(space, (from, was), at, route, now);
        let name = state.participants[participant].name.clone();
        let handle = crate::presence::handle(&name, state.participants[participant].tag);
        state.agent_pin(space, now, Some(handle));
        state
            .space_mut(space)
            .expect("found above")
            .following
            .partner = Some(participant.to_owned());
        log::info!("{name} called the agent of space {space} to them, where it stays.");
        Ok(())
    }

    /// Where the agent of `space` stands at `participant`'s conversation spot at `now`, as following would choose it, as
    /// its own `move_agent beside` asks: straight towards them from where it is, from where they stand now, else, where
    /// they have not reported, from `head`, where they were as their words began; or, with nowhere clear there, in front
    /// of them. With the eyes it faces.
    pub(super) fn beside_spot(
        &self,
        state: &HubState,
        space: &str,
        participant: Option<&str>,
        head: Pose,
        now: Instant,
    ) -> (Pose, [f32; 3]) {
        let captured = participant.and_then(|participant| self.capture(state, space, participant));
        let (speaker, scene) = match captured {
            Some(captured) => (captured.speaker, captured.scene),
            None => (Speaker { head }, self.scene(state, space).0),
        };
        let record = state.space(space).expect("the caller found it");
        let home = home(&self.environments().anchors(&record.model.environment));
        let from = record.following.at(record.summoned, home, now).position;
        let at = follow::spot(from, &speaker, &scene)
            .unwrap_or_else(|| crate::presence::before(&speaker.head));
        (at, speaker.head.position)
    }

    /// Where the agent of `space` stands to be beside what stands at `object`, for whoever asked, whose head was at
    /// `asker` as their words began: off their line of sight to it, at the conversation distance from where they stand
    /// or, further from it than [`follow::GAZE`], from where they would stand to look at it from that far, facing them;
    /// or none, where nowhere there is clear. See `design/environments.md`.
    pub(super) fn beside_object(
        &self,
        state: &HubState,
        space: &str,
        object: [f32; 3],
        asker: Pose,
    ) -> Option<Pose> {
        let eyes = asker.position;
        let (dx, dz) = (eyes[0] - object[0], eyes[2] - object[2]);
        let apart = dx.hypot(dz);
        let viewpoint = if apart > follow::GAZE {
            let share = follow::GAZE / apart;
            [object[0] + dx * share, eyes[1], object[2] + dz * share]
        } else {
            eyes
        };
        let head = Pose::upright_facing(viewpoint, [object[0], viewpoint[1], object[2]]);
        follow::beside(&head, object, &self.scene(state, space).0)
    }

    /// Move the agent of `space` to `at`, before someone whose eyes are at `eyes`, as its own `move_agent` asked, at
    /// `now`, its status wall and workers staying home: the caller found it stands elsewhere.
    pub(super) fn agent_goes(
        &self,
        state: &mut HubState,
        space: &str,
        at: Pose,
        eyes: [f32; 3],
        now: Instant,
    ) {
        let record = state.space(space).expect("the caller found it");
        let (scene, _, home) = self.scene(state, space);
        let from = record.following.at(record.summoned, home, now).position;
        let route = follow::route(from, at.position, &scene);
        let was = record.summoned;
        state.summon(space, Some(at), Some(eyes));
        state.agent_moved(space, (from, was), at, route, now);
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
    /// The agent of `space` keeps with nobody from now, their exchange with it over: logged, once, if it kept with
    /// someone.
    fn keep_with_nobody(&mut self, space: &str) {
        let record = self.space_mut(space).expect("the caller found it");
        let Some(keeping) = record.following.keeping.take() else {
            return;
        };
        let name = self
            .participants
            .get(&keeping.who)
            .map_or("someone", |participant| participant.name.as_str());
        log::info!(
            "The agent of space {space} keeps with {name} no more, as their exchange with it is over."
        );
    }

    /// How the agent of `space` moves changed, as `name` set it: kept home, it goes home now; otherwise it stays where it
    /// is, and follows from the next words if it now follows the speaker.
    pub(super) fn movement_changed(&mut self, space: &str, name: &str, now: Instant) {
        let record = self.space_mut(space).expect("the caller found it");
        if record.model.movement().mode == Movement::Home {
            self.agent_home(space, &format!("{name} keeps it home"), now);
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
        let eyes = record.partner;
        // Barely begun, it stays where it was; else it stops where it has reached, facing whoever it went to from
        // there, rather than as it would have faced them from where it was going, unless that is within [`FACE`] of
        // it, as it is coming straight towards them, so that it turns no more than it must.
        record.summoned = if follow::distance(reached, glide.from) < 0.05 {
            glide.was
        } else {
            record.summoned.map(|summoned| {
                let stopped = Pose {
                    position: reached,
                    ..summoned
                };
                match eyes {
                    Some(eyes)
                        if follow::apart(
                            follow::faces(&stopped),
                            follow::towards(reached, eyes),
                        ) > FACE.to_radians() =>
                    {
                        Pose::upright_facing(reached, [eyes[0], reached[1], eyes[2]])
                    }
                    _ => stopped,
                }
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
            // It keeps with them while it thinks and replies, unless their press was a tap.
            following.answering = (!tapped).then(|| participant.clone());
            if !tapped {
                self.follow(space, &participant, now, "it began answering them");
            }
        }
    }

    /// Go to `participant` in `space` at `now`, for `reason`, as following the speaker does, from what their last press
    /// captured; or only turn to them, where it already stands near enough, stays its [`DWELL`] for another, or finds no
    /// spot clear. Nothing while it is pinned, or in a space whose agent does not follow.
    fn follow(&mut self, space: &str, participant: &str, now: Instant, reason: &str) {
        let record = self.space_mut(space).expect("the caller found it");
        if record.model.movement().mode != Movement::Follow || record.following.pinned {
            return;
        }
        let members = record.model.participants.clone();
        let following = &record.following;
        let Some(captured) = following.captured.get(participant).cloned() else {
            return;
        };
        let eyes = captured.speaker.head.position;
        // Several who spoke to it lately, standing together, are served from one spot.
        let others: Vec<Speaker> = following
            .spoken
            .iter()
            .filter(|(who, at)| {
                who != participant
                    && now.saturating_duration_since(*at) < LATELY
                    && members.contains(who)
            })
            .filter_map(|(who, _)| following.captured.get(who))
            .map(|captured| captured.speaker)
            .filter(|other| follow::together(other.head.position, eyes))
            .collect();
        self.go_to(space, participant, &captured, others, now, reason);
    }

    /// Go to `participant` in `space` at `now`, for `reason`, where `captured` finds them, with `others` of a huddle
    /// standing together with them; or only turn to them, where it already stands near enough, stays its [`DWELL`] for
    /// another, or finds no spot clear. It keeps with them from here, through their exchange with it.
    fn go_to(
        &mut self,
        space: &str,
        participant: &str,
        captured: &Captured,
        others: Vec<Speaker>,
        now: Instant,
        reason: &str,
    ) {
        let name = self
            .participants
            .get(participant)
            .map(|record| record.name.clone())
            .unwrap_or_default();
        let record = self.space_mut(space).expect("the caller found it");
        let following = &mut record.following;
        let agent = following.at(record.summoned, captured.home, now);
        let speaker = captured.speaker;
        let eyes = speaker.head.position;
        let scene = &captured.scene;
        following.partner = Some(participant.to_owned());
        following.keeping = Some(Keeping {
            who: participant.to_owned(),
            at: eyes,
            heading: follow::heading(&speaker.head),
            since: None,
            due: None,
        });
        let turn = |following: &mut Following| {
            following.facing = Some(eyes);
            following.poll.get_or_insert(now + POLL);
        };
        if follow::near_enough(&agent, &speaker) {
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
        let mut group = vec![speaker];
        group.extend(others);
        let huddle = group.len() > 1;
        // Straight towards them from where it is, whichever way they face.
        let spot = if huddle {
            follow::huddle(agent.position, &group, scene)
        } else {
            follow::spot(agent.position, &speaker, scene)
        };
        let Some(spot) = spot else {
            turn(following);
            log::info!(
                "The agent of space {space} only turned to {name}, as {reason}, {:.1} m from them, as nowhere near them was clear.",
                follow::distance(agent.position, eyes)
            );
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
        let [x, y, z] = spot.position;
        let how = match route {
            Route::Straight => "gliding",
            Route::Via(_) => "gliding by a waypoint",
            Route::Stream => "streaming",
        };
        log::info!(
            "The agent of space {space} followed {with}, as {reason}, {:.1} m from them, {how} to ({x:.2}, {y:.2}, {z:.2}), {:.1} m from them, facing them.",
            follow::distance(agent.position, eyes),
            follow::distance(spot.position, eyes)
        );
        self.touch_space(space);
        self.woken.notify(usize::MAX);
    }

    /// Send the agent of `space` home at `now`, unpinned and facing as its home faces, for `reason`, logged if it
    /// moved; whether it moved. It glides home from wherever it has reached.
    pub(super) fn agent_home(&mut self, space: &str, reason: &str, now: Instant) -> bool {
        let record = self.space_mut(space).expect("the caller found it");
        let turned = record.following.faces().is_some() || record.following.pinned;
        let homing = match (record.summoned, &record.following.glide) {
            (None, _) => record.following.homing,
            (Some(_), Some(glide)) => Some((glide.at(now), now)),
            (Some(summoned), None) => Some((summoned.position, now)),
        };
        record.following = Following {
            homing,
            captured: std::mem::take(&mut record.following.captured),
            spoken: std::mem::take(&mut record.following.spoken),
            holding: std::mem::take(&mut record.following.holding),
            tapped: std::mem::take(&mut record.following.tapped),
            speaking: record.following.speaking,
            unspoken: record.following.unspoken,
            progressed: record.following.progressed,
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

    /// Pin the agent of `space` where it is going, or stands, so that it follows nobody and does not go home on its own,
    /// as the participant `by`, by their handle, or else the agent itself, asked: a pin a participant made stays theirs as the
    /// agent pins itself again.
    pub(super) fn agent_pin(&mut self, space: &str, now: Instant, by: Option<String>) {
        let following = &mut self
            .space_mut(space)
            .expect("the caller found it")
            .following;
        let theirs = following.pinned_by.take().filter(|_| following.pinned);
        following.pinned = true;
        following.pinned_by = by.or(theirs);
        following.pending = None;
        following.empty = None;
        following.poll.get_or_insert(now + POLL);
        self.touch_space(space);
        self.woken.notify(usize::MAX);
    }

    /// Pin the agent of `space` where it stands, or, `pinned` false, end its pin, as `participant` asked from the hand
    /// menu, with **Tomori: stay here** or **Tomori: follow again**, at `now`, whoever pinned it: a participant always
    /// wins. Refused where the space keeps it home, and ending a pin, unless it follows the speaker there.
    pub(super) fn pin_agent(
        &mut self,
        space: &str,
        participant: &str,
        pinned: bool,
        now: Instant,
    ) -> Result<(), &'static str> {
        let mode = self
            .space(space)
            .map(|record| record.model.movement().mode)
            .unwrap_or_default();
        if mode == Movement::Home {
            return Err(
                "Tomori stays home in this space: an owner can let it move in the Space section.",
            );
        }
        let name = self.participants[participant].name.clone();
        if pinned {
            let handle = crate::presence::handle(&name, self.participants[participant].tag);
            self.agent_pin(space, now, Some(handle));
            log::info!("{name} asked the agent of space {space} to stay where it stands.");
        } else if mode == Movement::Follow {
            self.agent_unpin(space, Some(&name), None, now);
        } else {
            return Err(
                "Tomori moves only when asked in this space, so it stays where it goes: an owner can let it follow in the Space section.",
            );
        }
        Ok(())
    }

    /// End the pin of the agent of `space` at `now`, as `name`, a participant, or else the agent itself, asked, so that it
    /// follows the speaker again: it goes home once the conversation is quiet, and, where `speaker`, the participant
    /// whose words it answers, said them as they held X, it goes to them now, as following would. Whether it was pinned.
    pub(super) fn agent_unpin(
        &mut self,
        space: &str,
        name: Option<&str>,
        speaker: Option<&str>,
        now: Instant,
    ) -> bool {
        let record = self.space_mut(space).expect("the caller found it");
        let following = &mut record.following;
        let pinned = std::mem::take(&mut following.pinned);
        following.pinned_by = None;
        // The way home is counted from the pin's end, and not put off again by asking to follow while it follows.
        if pinned {
            following.empty = None;
            following.quiet = None;
        }
        following.poll = Some(now + POLL);
        match name {
            Some(name) => {
                log::info!("{name} let the agent of space {space} follow the speaker again.");
            }
            None => {
                log::info!("The agent of space {space} follows the speaker again, as it asked.")
            }
        }
        if let Some(speaker) = speaker {
            self.follow(space, speaker, now, "it was asked to follow again");
        }
        self.touch_space(space);
        self.woken.notify(usize::MAX);
        pinned
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
        // Sent somewhere, it keeps with nobody until the next words that move it.
        following.keeping = None;
        following.poll.get_or_insert(now + POLL);
        self.woken.notify(usize::MAX);
    }

    /// Whether the conversation in `space` is quiet at `now`: nobody records or is transcribed for it, nothing is queued,
    /// no approval or question waits, its agent's session is idle, and no agent speech plays, is held for anyone or is
    /// still to come from synthesis.
    fn conversation_quiet(&self, space: &str, now: Instant) -> bool {
        let record = self.space(space).expect("the caller found it");
        !self
            .utterances
            .values()
            .any(|utterance| utterance.space == space)
            && record.model.waiting() == 0
            && matches!(record.model.activity(), Activity::Idle | Activity::Unknown)
            && record.speech.quiet()
            && !record.following.unspoken(now)
            && record.following.speaking.is_none_or(|until| until <= now)
    }

    /// Whether the agent of `space` is still replying at `now`, to whoever's words it was given last: their words or
    /// another's being given or queued, its turn running or a question or approval of its waiting, or its speech
    /// playing, held for anyone or still to come, queued for synthesis after its turn has ended, until the last of it
    /// has played. Its voice, not its text turn, ends its reply. See `design/environments.md`.
    fn replying(&self, space: &str, now: Instant) -> bool {
        let record = self.space(space).expect("the caller found it");
        record.model.waiting() > 0
            || matches!(
                record.model.activity(),
                Activity::Running | Activity::Waiting
            )
            || !record.speech.quiet()
            || record.following.unspoken(now)
            || record.following.speaking.is_some_and(|until| until > now)
    }

    /// Whom the agent of `space` keeps with at `now`, if anyone: the partner it went to, or turned to, for their words,
    /// while they still speak to it, holding X past a tap or being transcribed, and while it thinks and replies to them,
    /// in a space whose agent follows the speaker and is not pinned. Never a visitor, nor anyone after it was sent
    /// somewhere.
    pub(super) fn keeps_with(&self, space: &str, now: Instant) -> Option<String> {
        let record = self.space(space).expect("the caller found it");
        let following = &record.following;
        let who = &following.keeping.as_ref()?.who;
        let speaking = self.utterances.values().any(|utterance| {
            utterance.space == space
                && utterance.participant == *who
                && utterance.answer.is_none()
                && !utterance.naming
                && !utterance.titling
        }) && !following.holding.contains_key(who)
            && !following.tapped.contains(who);
        let answering = following.answering.as_ref() == Some(who) && self.replying(space, now);
        (record.model.movement().mode == Movement::Follow
            && !following.pinned
            && following.partner.as_ref() == Some(who)
            && record.model.participants.contains(who)
            && (speaking || answering))
            .then(|| who.clone())
    }

    /// When the agent of any space next does something, if any does: commits to a press, stops glancing, arrives,
    /// checks whether to go home or whether to follow on whoever it keeps with.
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

    /// Whether the partner of the agent of `space` has left it, or gone away.
    pub(super) fn partner_gone(&self, space: &str) -> bool {
        let record = self.space(space).expect("the caller found it");
        record.following.partner.as_ref().is_some_and(|partner| {
            !record.model.participants.contains(partner)
                || self.participants.get(partner).is_some_and(|participant| {
                    participant.engagement == Some(tomori_protocol::Engagement::Away)
                })
        })
    }

    fn movement_due(&mut self, space: &str, now: Instant) {
        let quiet = self.conversation_quiet(space, now);
        let replying = self.replying(space, now);
        let gone = self.partner_gone(space);
        let record = self.space_mut(space).expect("found above");
        let setting = record.model.movement();
        let empty = record.model.participants.is_empty();
        let away = record.summoned.is_some();
        let following = &mut record.following;
        let mut changed = false;
        // Its reply over, it keeps with them no more, unless they speak to it again.
        if !replying {
            following.answering = None;
        }
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
            self.agent_home(space, reason, now);
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
        self.homing = None;
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
