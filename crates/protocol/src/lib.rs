// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Wire protocol shared by Tomori clients and the Tomori server.
//!
//! Each client keeps one long-lived, bidirectional connection to the server.
//! Both directions carry a sequence of [`Frame`]s: JSON control messages
//! ([`ClientMessage`] and [`ServerMessage`]) and binary [`AudioFrame`]s. See
//! `design/server.md`.
mod audio;
mod cards;
mod channel;
pub mod environments;
mod frame;
mod keepalive;
mod message;
mod presence;
mod space;
pub mod text;

pub use audio::{AGENT_SOURCE, AUDIO_HEADER_BYTES, AudioError, AudioFrame, Gate, REPLAY_SOURCE};
pub use cards::{Card, CardKind, Cards, Checks, Label, MAX_CARDS};
pub use channel::{MESSAGE_LIMIT, Payload, STATE_LIMIT};
pub use frame::{Frame, FrameDecoder, FrameError, FrameKind, MAX_FRAME_BYTES};
pub use keepalive::Keepalive;
pub use message::{
    ATTENTION_LIMIT, AnswerTarget, ClientMessage, Decision, ENVIRONMENT_LIMIT, Pose, Resource,
    ServerMessage, TranscriptPart, TranscriptStage,
};
pub use presence::{BONES, Hand, JOINT_COUNT, Joint};
pub use space::{
    Activity, AgentInstructions, Approval, ApprovalOption, Attention, BackendInfo, ConnectionRole,
    Engagement, EnvironmentInfo, EnvironmentVersion, KnockRequest, KnockState, Message, Mode,
    Participant, Presentation, PresentationKind, Question, QueueEntry, QueueState, Role,
    SessionSummary, SpaceState, SpaceSummary, SpeechSetting, Worker, WorkerState, WorkspaceRoot,
};

/// The protocol version sent in [`ClientMessage::Hello`] and [`ServerMessage::Welcome`].
pub const PROTOCOL_VERSION: u32 = 2;

/// The HTTP path that clients use to open their connection.
pub const CONNECT_PATH: &str = "/connect";

/// The media type of a connection body in either direction.
pub const CONTENT_TYPE: &str = "application/x-tomori-frames";

/// The default server port.
pub const DEFAULT_PORT: u16 = 8765;
