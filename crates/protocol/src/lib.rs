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
mod frame;
mod keepalive;
mod message;
mod presence;
mod projection;
mod space;
pub mod text;

pub use audio::{AGENT_SOURCE, AUDIO_HEADER_BYTES, AudioError, AudioFrame, Gate, REPLAY_SOURCE};
pub use cards::{Card, CardKind, Cards, Checks, Label, MAX_CARDS};
pub use channel::{MESSAGE_LIMIT, Payload, STATE_LIMIT};
pub use frame::{Frame, FrameDecoder, FrameError, FrameKind, MAX_FRAME_BYTES};
pub use keepalive::Keepalive;
pub use message::{
    ATTENTION_LIMIT, AnswerTarget, ClientMessage, Decision, ENVIRONMENT_LIMIT, NO_SPEECH,
    PAIRING_CLOSED, Pose, Resource, ServerMessage, SpaceDeletion, TranscriptPart, TranscriptStage,
};
pub use presence::{BONES, Hand, JOINT_COUNT, Joint};
pub use projection::layout as wall;
pub use projection::{
    CODE_CHARS, CODE_LINES, CODE_SPANS, Code, Grid, IMAGE_BYTES, IMAGE_CHUNK_BYTES, IMAGE_SIDE,
    Image, ImageChunk, ImageChunkError, Meta, Panel, Place, Projection, SlotFrame, Span,
    TOMORI_WALL, TokenKind, WALL_SLOTS, Wall, WallKind, WallShape, hash_bytes, hash_text,
    valid_spans,
};
pub use space::{
    Activity, AgentInstructions, Approval, ApprovalOption, Attention, BackendInfo, ConnectionRole,
    Engagement, EnvironmentInfo, EnvironmentVersion, KnockRequest, KnockState, Message, Mode,
    Participant, Presentation, PresentationKind, Question, QueueEntry, QueueState, Renamer, Role,
    SessionSummary, SpaceState, SpaceSummary, SpeechSetting, Worker, WorkerState, WorkspaceRoot,
};

/// The identifier of the Minimal grid, the plain floor grid that every client draws, built in: outside any space, and in a space whose environment it cannot present at the version the space names. It is the protocol's fallback rather than an environment of its own, so its identity is here; every other environment, Tomori's own included, defines its identity in its own crate, and an application registers it. A server offers the grid only when it registers it, as Tomori's own does. See `design/environments.md`.
pub const GRID: &str = "minimal";

/// The protocol version of the Minimal grid, [`GRID`], which every client presents it at.
pub const GRID_VERSION: u32 = 1;

/// The protocol version sent in [`ClientMessage::Hello`] and [`ServerMessage::Welcome`].
pub const PROTOCOL_VERSION: u32 = 2;

/// The HTTP path that clients use to open their connection.
pub const CONNECT_PATH: &str = "/connect";

/// The media type of a connection body in either direction.
pub const CONTENT_TYPE: &str = "application/x-tomori-frames";

/// The default server port.
pub const DEFAULT_PORT: u16 = 8765;
