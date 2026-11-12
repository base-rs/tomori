// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the server takes from each client: frames no longer, and no more often, than the client's role sends,
//! whatever the connection's frame limit for what the server sends it. See `design/security.md`.
//!
//! TODO(upstream): bytes a second of a body, connections per address and requests per connection are the HTTP
//! server's to bound, base-rs/base-http#74; pacing here sees only frames.
use std::time::Instant;

use tomori_protocol::FrameKind;

use crate::access::{Access, Grant};

/// The longest frame a connection that only watches sends: a visitor's, one admitted only to pair, and any connection
/// before its Hello. The longest it sends is its Hello, of a few hundred bytes, or a few KiB for a client that
/// presents many environments; what else it may send, such as a join, a knock, a rename or asking for an image or the
/// projection whole, is smaller.
pub const WATCHING_FRAME_BYTES: usize = 16 * 1024;

/// The longest frame an owner's or a collaborator's connection sends, after its Hello. The longest a client sends is
/// about 20 KiB, a space created with the longest directory and instructions the server takes, escaped in JSON, and
/// the most frequent the 1,944 bytes of 20 ms of audio at 48 kHz and the few KiB of presence with both hands tracked.
pub const PARTICIPATING_FRAME_BYTES: usize = 256 * 1024;

/// What a connection is taken as, for what it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inbound {
    /// A visitor's, a connection admitted only to pair, or any before its Hello.
    Watching,
    /// An owner's or a collaborator's, after its Hello. A collaborator is one only in their own space, but is taken as
    /// participating wherever they are, as admitted with a token.
    Participating,
}

impl Inbound {
    /// What a connection admitted as `access` is taken as once it has said Hello.
    pub fn greeted(access: &Access) -> Self {
        match access.grant {
            Grant::Owner | Grant::Collaborator { .. } => Self::Participating,
            Grant::Visitor | Grant::Pairing => Self::Watching,
        }
    }

    /// The longest frame taken.
    pub const fn frame_bytes(self) -> usize {
        match self {
            Self::Watching => WATCHING_FRAME_BYTES,
            Self::Participating => PARTICIPATING_FRAME_BYTES,
        }
    }

    /// Frames taken each second, and how many beyond that may come at once.
    const fn pace(self) -> (f64, f64) {
        match self {
            Self::Watching => (WATCHING_PER_SECOND, WATCHING_BURST),
            Self::Participating => (PARTICIPATING_PER_SECOND, PARTICIPATING_BURST),
        }
    }

    /// Whether a frame of `kind` counts against [`Pace`]. A participant's audio is paced by their microphone, 50
    /// frames a second, but may come in a burst of many seconds of an utterance held while the link was congested, so
    /// that only their control messages are counted; a watching connection sends no audio, so all it sends is.
    fn counts(self, kind: FrameKind) -> bool {
        self == Self::Watching || kind == FrameKind::Control
    }

    /// Who is refused, in what the client is told.
    pub fn whom(self) -> &'static str {
        match self {
            Self::Watching => "a visitor, a device pairing or a client before its Hello",
            Self::Participating => "an owner or a collaborator",
        }
    }
}

/// Frames a watching connection sends each second, at most, sustained: it sends a few on joining a space, and then
/// keepalive and the images and projection it asks for, a few at a time.
const WATCHING_PER_SECOND: f64 = 32.0;

/// Frames a watching connection may send at once beyond [`WATCHING_PER_SECOND`].
const WATCHING_BURST: f64 = 256.0;

/// Control messages a participant's connection sends each second, at most, sustained: presence once each frame it
/// draws, at most 120 a second on a headset and the display's rate, perhaps 360, on a desktop, and a few others.
const PARTICIPATING_PER_SECOND: f64 = 1000.0;

/// Control messages a participant's connection may send at once beyond [`PARTICIPATING_PER_SECOND`], as many as a
/// client may hold while its link is congested.
const PARTICIPATING_BURST: f64 = 1000.0;

/// What becomes of a frame, as [`Pace::take`] decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Applied.
    Take,
    /// Dropped, unread, since the connection is sending more often than it may.
    Drop,
    /// The connection has sent far more often than it may for long enough that it is closed.
    Refuse,
}

/// A token bucket: as many tokens as its burst, refilled at its rate.
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Option<Instant>,
}

impl Bucket {
    const fn new() -> Self {
        Self {
            tokens: 0.0,
            last: None,
        }
    }

    /// Take a token at `now` from a bucket of `burst`, refilled at `rate` a second, if it holds one.
    fn draw(&mut self, rate: f64, burst: f64, now: Instant) -> bool {
        self.tokens = match self.last {
            Some(last) => {
                let elapsed = now.saturating_duration_since(last).as_secs_f64();
                (self.tokens + elapsed * rate).min(burst)
            }
            None => burst,
        };
        self.last = Some(now.max(self.last.unwrap_or(now)));
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// How often a connection sends, as two token buckets. A frame is taken while the first, of its burst refilled at its
/// rate, holds a token, and dropped otherwise, so that a connection sending more often loses only what is past its
/// rate. Every frame counted draws on the second too, of twice its burst refilled at half again its rate, so that one
/// that keeps on sending at half again its rate or more, beyond twice its burst, is closed, within seconds.
/// Each starts full when the connection is first taken as what it is, as when its Hello or a knock admits it as a
/// participant, so that what it sent as a client before its Hello, or as a visitor, takes nothing from what it may
/// send as a participant.
#[derive(Debug, Default)]
pub struct Pace {
    take: Bucket,
    close: Bucket,
    inbound: Option<Inbound>,
}

impl Default for Bucket {
    fn default() -> Self {
        Self::new()
    }
}

impl Pace {
    pub fn new() -> Self {
        Self::default()
    }

    /// What becomes of a frame of `kind`, arriving `now` on a connection taken as `inbound`.
    pub fn take(&mut self, inbound: Inbound, kind: FrameKind, now: Instant) -> Verdict {
        if !inbound.counts(kind) {
            return Verdict::Take;
        }
        if self.inbound != Some(inbound) {
            *self = Self {
                inbound: Some(inbound),
                ..Self::default()
            };
        }
        let (rate, burst) = inbound.pace();
        if !self.close.draw(rate * 1.5, burst * 2.0, now) {
            Verdict::Refuse
        } else if self.take.draw(rate, burst, now) {
            Verdict::Take
        } else {
            Verdict::Drop
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tomori_protocol::{AudioFrame, ClientMessage, Frame, Gate, Pose};

    use super::*;

    /// The frames each role takes, from the longest each sends, measured: a Hello presenting the most environments,
    /// a space created with the longest of everything, presence with both hands tracked and 20 ms of audio at 48 kHz.
    #[test]
    fn the_longest_frame_each_role_sends_is_taken_with_room_to_spare() {
        let identifier = "a".repeat(64);
        let hello = ClientMessage::Hello {
            protocol: tomori_protocol::PROTOCOL_VERSION,
            participant: "p".repeat(crate::limits::PARTICIPANT_BYTES),
            name: "n".repeat(crate::limits::NAME_BYTES),
            names: true,
            environments: Some(
                (0..tomori_protocol::ENVIRONMENT_LIMIT)
                    .map(|_| tomori_protocol::EnvironmentVersion {
                        id: identifier.clone(),
                        version: u32::MAX,
                    })
                    .collect(),
            ),
            environment_state: true,
            projection: true,
            pairing_window: true,
            speech_held: true,
            images: true,
            diagrams: true,
            projection_patches: true,
            live_panels: true,
            limits: Some(tomori_protocol::Limits::MOST),
        };
        let hello = Frame::client(&hello).payload.len();
        assert!(hello * 2 < WATCHING_FRAME_BYTES, "{hello} bytes");
        // Quotes and line breaks are escaped in two bytes each, as are most of what instructions hold.
        let create = ClientMessage::CreateSpace {
            title: "t".repeat(crate::limits::TITLE_BYTES),
            backend: identifier.clone(),
            directory: "\"".repeat(crate::limits::DIRECTORY_BYTES),
            session: Some("s".repeat(crate::limits::SESSION_BYTES)),
            environment: Some(identifier),
            instructions: Some("\n".repeat(crate::limits::INSTRUCTIONS_BYTES)),
        };
        let create = Frame::client(&create).payload.len();
        let joint = tomori_protocol::Joint {
            position: [-1.234_567_8; 3],
            radius: 0.012_345_678,
        };
        let hand = tomori_protocol::Hand {
            joints: [Some(joint); tomori_protocol::JOINT_COUNT],
        };
        let pose = Pose {
            position: [-1.234_567_8; 3],
            orientation: [-0.123_456_78; 4],
        };
        let presence = ClientMessage::Presence {
            head: pose,
            left: Some(pose),
            right: Some(pose),
            hands: Some(Box::new([Some(hand), Some(hand)])),
            carrying: Some("panel-1".repeat(20)),
            carried: Vec::new(),
            rays: None,
        };
        let presence = Frame::client(&presence).payload.len();
        let audio = Frame::audio(&AudioFrame {
            gate: Gate::Agent,
            source: 0,
            sequence: 0,
            timestamp: 0,
            sample_rate: 48_000,
            samples: vec![0; 960],
        })
        .payload
        .len();
        assert_eq!(audio, 1_944);
        let longest = [hello, create, presence, audio].into_iter().max().unwrap();
        assert!(longest * 8 < PARTICIPATING_FRAME_BYTES, "{longest} bytes");
        assert!(
            create < 32 * 1024 && presence < 16 * 1024,
            "{create}, {presence}"
        );
    }

    #[test]
    fn owners_and_collaborators_participate_and_others_watch() {
        assert_eq!(Inbound::greeted(&Access::open()), Inbound::Participating);
        let collaborator = Access {
            principal: Some("token:1".into()),
            name: None,
            grant: Grant::Collaborator {
                space: "studio".into(),
            },
        };
        assert_eq!(Inbound::greeted(&collaborator), Inbound::Participating);
        assert_eq!(Inbound::greeted(&Access::visitor()), Inbound::Watching);
        assert_eq!(Inbound::greeted(&Access::pairing()), Inbound::Watching);
        assert!(Inbound::Watching.frame_bytes() < Inbound::Participating.frame_bytes());
        assert!(Inbound::Participating.frame_bytes() < tomori_protocol::MAX_FRAME_BYTES);
        assert_ne!(Inbound::Watching.whom(), Inbound::Participating.whom());
    }

    /// Frames sent at `per_second` for `seconds`, and what became of them.
    fn sent(
        pace: &mut Pace,
        inbound: Inbound,
        kind: FrameKind,
        start: Instant,
        per_second: u32,
        seconds: u32,
    ) -> (usize, usize, usize) {
        let mut verdicts = (0, 0, 0);
        for frame in 0..per_second * seconds {
            let now = start + Duration::from_secs_f64(f64::from(frame) / f64::from(per_second));
            match pace.take(inbound, kind, now) {
                Verdict::Take => verdicts.0 += 1,
                Verdict::Drop => verdicts.1 += 1,
                Verdict::Refuse => {
                    verdicts.2 += 1;
                    break;
                }
            }
        }
        verdicts
    }

    #[test]
    fn a_connection_sending_within_its_pace_loses_nothing() {
        let start = Instant::now();
        // Presence at a desktop's 360 Hz, for a minute.
        let mut pace = Pace::new();
        let participating = Inbound::Participating;
        assert_eq!(
            sent(&mut pace, participating, FrameKind::Control, start, 360, 60),
            (360 * 60, 0, 0)
        );
        // A burst of everything held while congested, at once.
        let mut pace = Pace::new();
        let mut taken = 0;
        for _ in 0..1000 {
            taken +=
                usize::from(pace.take(participating, FrameKind::Control, start) == Verdict::Take);
        }
        assert_eq!(taken, 1000);
        // A participant's audio is not counted, however much comes at once.
        let mut pace = Pace::new();
        for _ in 0..100_000 {
            assert_eq!(
                pace.take(participating, FrameKind::Audio, start),
                Verdict::Take
            );
        }
        // A visitor joining and fetching images, a few at a time, for a minute.
        let mut pace = Pace::new();
        assert_eq!(
            sent(
                &mut pace,
                Inbound::Watching,
                FrameKind::Control,
                start,
                20,
                60
            ),
            (20 * 60, 0, 0)
        );
    }

    #[test]
    fn a_connection_sending_past_its_pace_loses_what_is_past_it_and_is_closed_if_it_keeps_on() {
        let start = Instant::now();
        // A visitor's flood: its burst taken, as many again dropped, then refused.
        let mut pace = Pace::new();
        let mut verdicts = Vec::new();
        for _ in 0..1000 {
            let verdict = pace.take(Inbound::Watching, FrameKind::Audio, start);
            verdicts.push(verdict);
            if verdict == Verdict::Refuse {
                break;
            }
        }
        let count = |wanted: Verdict| {
            verdicts
                .iter()
                .filter(|verdict| **verdict == wanted)
                .count()
        };
        assert_eq!(count(Verdict::Take), 256);
        assert_eq!(count(Verdict::Drop), 256);
        assert_eq!(count(Verdict::Refuse), 1);
        // Pausing fills it again, never beyond its burst.
        let mut pace = Pace::new();
        for _ in 0..300 {
            pace.take(Inbound::Watching, FrameKind::Control, start);
        }
        let later = start + Duration::from_secs(60);
        assert_eq!(
            sent(
                &mut pace,
                Inbound::Watching,
                FrameKind::Control,
                later,
                1_000_000,
                1
            )
            .0,
            256
        );
        // A participant sending at a quarter again their rate loses only what is past it, a fifth of what they send.
        let mut pace = Pace::new();
        let (taken, dropped, refused) = sent(
            &mut pace,
            Inbound::Participating,
            FrameKind::Control,
            start,
            1250,
            60,
        );
        assert_eq!(refused, 0);
        assert!((60_900..61_100).contains(&taken), "{taken}");
        assert!((13_900..14_100).contains(&dropped), "{dropped}");
        // At twice it, they are closed within four seconds, once twice their burst is past half again their rate,
        // having lost only what was past their rate.
        let mut pace = Pace::new();
        let (taken, dropped, refused) = sent(
            &mut pace,
            Inbound::Participating,
            FrameKind::Control,
            start,
            2000,
            10,
        );
        assert_eq!(refused, 1);
        assert!((4_900..5_100).contains(&taken), "{taken}");
        assert!((2_900..3_100).contains(&dropped), "{dropped}");
        // A visitor admitted as a collaborator has a participant's burst at once, whatever they sent before.
        let mut pace = Pace::new();
        for _ in 0..512 {
            pace.take(Inbound::Watching, FrameKind::Control, start);
        }
        assert_eq!(
            pace.take(Inbound::Watching, FrameKind::Control, start),
            Verdict::Refuse
        );
        for _ in 0..1000 {
            assert_eq!(
                pace.take(Inbound::Participating, FrameKind::Control, start),
                Verdict::Take
            );
        }
        assert_eq!(
            pace.take(Inbound::Participating, FrameKind::Control, start),
            Verdict::Drop
        );
        // A frame from before the last, as from a clock read out of order, refills nothing.
        let mut pace = Pace::new();
        pace.take(Inbound::Watching, FrameKind::Control, later);
        assert_eq!(
            pace.take(Inbound::Watching, FrameKind::Control, start),
            Verdict::Take
        );
        assert_eq!(pace.take.tokens, WATCHING_BURST - 2.0);
    }
}
