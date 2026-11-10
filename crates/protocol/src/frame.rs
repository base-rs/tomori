// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fmt;

use crate::{AudioError, AudioFrame, ClientMessage, ImageChunk, ImageChunkError, ServerMessage};

/// The largest payload that every peer takes, the frame limit before limits were stated: a connection is bound by it
/// until both sides state more, in [`crate::Limits::frame_bytes`], and with a peer that states none, as an older one,
/// for as long as it lasts. No side states less. See `design/server.md`.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// The largest payload any connection takes, however much either side states: what Tomori's own clients and server
/// state, so that a projection at the caps they state fits in one frame. A side stating more is read as stating this.
pub const MOST_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// A frame whose payload is longer than this is read by a [`FrameDecoder`] straight into a payload of its own, as it
/// arrives, rather than into the decoder's buffer and then copied, so that its bytes are held once.
const DIRECT_BYTES: usize = 64 * 1024;

const HEADER_BYTES: usize = 5;

/// `limit` within what any connection takes: from [`MAX_FRAME_BYTES`], which every peer takes, to [`MOST_FRAME_BYTES`].
pub fn frame_limit(limit: usize) -> usize {
    limit.clamp(MAX_FRAME_BYTES, MOST_FRAME_BYTES)
}

/// The payload type of a [`Frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    /// A UTF-8 JSON control message.
    Control = 0,
    /// A binary [`AudioFrame`].
    Audio = 1,
    /// A binary [`ImageChunk`], part of an image panel's image, sent only to a client whose `Hello` sets `images`, as it
    /// asked for it with [`ClientMessage::FetchImage`]. See `design/projection.md`.
    Image = 2,
}

/// One unit of a connection body: a kind byte, a little-endian `u32` payload length, then the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameKind,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub enum FrameError {
    /// The kind byte is not a known [`FrameKind`].
    UnknownKind(u8),
    /// The payload, or the length a frame's header declares, exceeds the connection's frame limit: [`MAX_FRAME_BYTES`]
    /// unless both sides stated more.
    TooLarge(usize),
    /// A control payload is not a valid message.
    Json(serde_json::Error),
    /// An audio payload is malformed.
    Audio(AudioError),
    /// An image payload is malformed.
    Image(ImageChunkError),
    /// The frame has a different kind than the caller expected.
    UnexpectedKind(FrameKind),
    /// The stream ended part way through a frame.
    Truncated,
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind(kind) => write!(formatter, "unknown frame kind {kind}"),
            Self::TooLarge(length) => {
                write!(formatter, "frame of {length} bytes exceeds the limit")
            }
            Self::Json(error) => write!(formatter, "invalid control message: {error}"),
            Self::Audio(error) => write!(formatter, "invalid audio frame: {error}"),
            Self::Image(error) => write!(formatter, "invalid image frame: {error}"),
            Self::UnexpectedKind(kind) => write!(formatter, "unexpected {kind:?} frame"),
            Self::Truncated => write!(formatter, "connection ended inside a frame"),
        }
    }
}

impl std::error::Error for FrameError {}

impl Frame {
    pub fn control(message: &impl serde::Serialize) -> Self {
        Self {
            kind: FrameKind::Control,
            // Serializing our own message types cannot fail: they contain no maps with non-string keys.
            payload: serde_json::to_vec(message).expect("control messages serialize"),
        }
    }

    pub fn client(message: &ClientMessage) -> Self {
        Self::control(message)
    }

    pub fn server(message: &ServerMessage) -> Self {
        Self::control(message)
    }

    pub fn audio(frame: &AudioFrame) -> Self {
        Self {
            kind: FrameKind::Audio,
            payload: frame.encode(),
        }
    }

    pub fn image(chunk: &ImageChunk) -> Self {
        Self {
            kind: FrameKind::Image,
            payload: chunk.encode(),
        }
    }

    /// Append the encoded frame to `output`, or return [`FrameError::TooLarge`] without changing `output` if the payload exceeds [`MAX_FRAME_BYTES`], which every peer takes.
    pub fn try_encode_into(&self, output: &mut Vec<u8>) -> Result<(), FrameError> {
        self.try_encode_within(output, MAX_FRAME_BYTES)
    }

    /// Append the encoded frame to `output`, or return [`FrameError::TooLarge`] without changing `output` if the payload exceeds `limit`, the frame limit of the connection it is sent on.
    pub fn try_encode_within(&self, output: &mut Vec<u8>, limit: usize) -> Result<(), FrameError> {
        if self.payload.len() > limit {
            return Err(FrameError::TooLarge(self.payload.len()));
        }
        output.reserve(HEADER_BYTES + self.payload.len());
        output.push(self.kind as u8);
        output.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        output.extend_from_slice(&self.payload);
        Ok(())
    }

    /// Append `message` to `output` as a control frame, written straight into it rather than into a payload of its own
    /// first, so that a large message, such as a projection, is held once; or return [`FrameError::TooLarge`] without
    /// changing `output` if it is longer than `limit`, the frame limit of the connection it is sent on.
    pub fn encode_control_within(
        message: &impl serde::Serialize,
        output: &mut Vec<u8>,
        limit: usize,
    ) -> Result<(), FrameError> {
        let start = output.len();
        output.push(FrameKind::Control as u8);
        output.extend_from_slice(&[0; HEADER_BYTES - 1]);
        // Serializing our own message types cannot fail: they contain no maps with non-string keys.
        serde_json::to_writer(&mut *output, message).expect("control messages serialize");
        let length = output.len() - start - HEADER_BYTES;
        if length > limit {
            output.truncate(start);
            return Err(FrameError::TooLarge(length));
        }
        output[start + 1..start + HEADER_BYTES].copy_from_slice(&(length as u32).to_le_bytes());
        Ok(())
    }

    /// The encoded frame, or [`FrameError::TooLarge`] if the payload exceeds [`MAX_FRAME_BYTES`].
    pub fn try_encode(&self) -> Result<Vec<u8>, FrameError> {
        let mut output = Vec::new();
        self.try_encode_into(&mut output)?;
        Ok(output)
    }

    /// Append the encoded frame to `output`.
    ///
    /// # Panics
    ///
    /// If the payload exceeds [`MAX_FRAME_BYTES`]. Use this only where the payload is bounded by construction; otherwise use [`try_encode_into`](Self::try_encode_into). An oversized frame is never produced.
    pub fn encode_into(&self, output: &mut Vec<u8>) {
        if let Err(error) = self.try_encode_into(output) {
            panic!("{error}");
        }
    }

    /// The encoded frame. Panics like [`encode_into`](Self::encode_into).
    pub fn encode(&self) -> Vec<u8> {
        let mut output = Vec::new();
        self.encode_into(&mut output);
        output
    }

    pub fn to_client_message(&self) -> Result<ClientMessage, FrameError> {
        self.json()
    }

    pub fn to_server_message(&self) -> Result<ServerMessage, FrameError> {
        self.json()
    }

    pub fn to_audio(&self) -> Result<AudioFrame, FrameError> {
        if self.kind != FrameKind::Audio {
            return Err(FrameError::UnexpectedKind(self.kind));
        }
        AudioFrame::decode(&self.payload).map_err(FrameError::Audio)
    }

    pub fn to_image(&self) -> Result<ImageChunk, FrameError> {
        if self.kind != FrameKind::Image {
            return Err(FrameError::UnexpectedKind(self.kind));
        }
        ImageChunk::decode(&self.payload).map_err(FrameError::Image)
    }

    fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, FrameError> {
        if self.kind != FrameKind::Control {
            return Err(FrameError::UnexpectedKind(self.kind));
        }
        serde_json::from_slice(&self.payload).map_err(FrameError::Json)
    }
}

/// How much of a frame a [`FrameDecoder`] has received, while it is part way through one: of its payload, `received`
/// bytes of the `length` its header declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub kind: FrameKind,
    pub received: usize,
    pub length: usize,
}

/// A long frame being read straight into its payload; see [`DIRECT_BYTES`].
struct Direct {
    kind: FrameKind,
    length: usize,
    payload: Vec<u8>,
}

/// Incrementally split a byte stream into frames. Input can arrive in arbitrary chunks.
///
/// A frame longer than its limit, [`MAX_FRAME_BYTES`] unless [`set_limit`](Self::set_limit) says otherwise, is refused
/// from its header. A long one is read straight into its own payload, which grows as its bytes arrive, never beyond the
/// length its header declares, so that a header declaring a long frame allocates no more than has arrived of it, and
/// the frame is held once.
pub struct FrameDecoder {
    buffer: Vec<u8>,
    offset: usize,
    limit: usize,
    direct: Option<Direct>,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::with_limit(MAX_FRAME_BYTES)
    }
}

impl FrameDecoder {
    /// A decoder that takes frames of up to [`MAX_FRAME_BYTES`], as every peer does until both sides state more.
    pub fn new() -> Self {
        Self::default()
    }

    /// A decoder that takes frames of up to `limit`, within [`frame_limit`].
    pub fn with_limit(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            offset: 0,
            limit: frame_limit(limit),
            direct: None,
        }
    }

    /// The longest payload taken.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Take frames of up to `limit`, within [`frame_limit`], from the next whose header is read: as when both sides of a
    /// connection have stated their limits, after the frame that stated the peer's.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = frame_limit(limit);
    }

    pub fn push(&mut self, mut data: &[u8]) {
        if let Some(direct) = &mut self.direct {
            let taken = data.len().min(direct.length - direct.payload.len());
            let payload = &mut direct.payload;
            // Doubling, as a vector grows, but never beyond the declared length.
            let needed = payload.len() + taken;
            if needed > payload.capacity() {
                let target = (payload.capacity() * 2).clamp(needed, direct.length);
                payload.reserve_exact(target - payload.len());
            }
            payload.extend_from_slice(&data[..taken]);
            data = &data[taken..];
            if data.is_empty() {
                return;
            }
        }
        // Compact consumed input before growing, so a long-lived connection does not accumulate it.
        if self.offset > 0 && self.offset == self.buffer.len() {
            self.buffer.clear();
            self.offset = 0;
        } else if self.offset > 64 * 1024 {
            self.buffer.drain(..self.offset);
            self.offset = 0;
        }
        self.buffer.extend_from_slice(data);
    }

    /// Return the next complete frame, or `None` if more input is needed.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if let Some(direct) = &self.direct {
            if direct.payload.len() < direct.length {
                return Ok(None);
            }
            let direct = self.direct.take().expect("a frame being read");
            return Ok(Some(Frame {
                kind: direct.kind,
                payload: direct.payload,
            }));
        }
        let available = &self.buffer[self.offset..];
        let Some((kind, length)) = self.header()? else {
            return Ok(None);
        };
        if available.len() < HEADER_BYTES + length {
            if length > DIRECT_BYTES {
                // What is buffered of it so far is moved once into its own payload, and the rest read straight in.
                let received = &available[HEADER_BYTES..];
                let payload = received.to_vec();
                self.offset = self.buffer.len();
                self.direct = Some(Direct {
                    kind,
                    length,
                    payload,
                });
            }
            return Ok(None);
        }
        let payload = available[HEADER_BYTES..HEADER_BYTES + length].to_vec();
        self.offset += HEADER_BYTES + length;
        Ok(Some(Frame { kind, payload }))
    }

    /// The kind and length of the frame whose header is buffered, refusing a kind not known or a length beyond the limit.
    fn header(&self) -> Result<Option<(FrameKind, usize)>, FrameError> {
        let available = &self.buffer[self.offset..];
        if available.len() < HEADER_BYTES {
            return Ok(None);
        }
        let kind = match available[0] {
            0 => FrameKind::Control,
            1 => FrameKind::Audio,
            2 => FrameKind::Image,
            kind => return Err(FrameError::UnknownKind(kind)),
        };
        let length = u32::from_le_bytes(available[1..HEADER_BYTES].try_into().unwrap()) as usize;
        if length > self.limit {
            return Err(FrameError::TooLarge(length));
        }
        Ok(Some((kind, length)))
    }

    /// How much of the frame being received has arrived, once its header has and until all of it has; `None` between
    /// frames. A client shows it while a long frame arrives, as the projection whole on joining a space.
    pub fn progress(&self) -> Option<Progress> {
        if let Some(direct) = &self.direct {
            return Some(Progress {
                kind: direct.kind,
                received: direct.payload.len(),
                length: direct.length,
            });
        }
        let Ok(Some((kind, length))) = self.header() else {
            return None;
        };
        let received = self.buffer.len() - self.offset - HEADER_BYTES;
        (received < length).then_some(Progress {
            kind,
            received,
            length,
        })
    }

    /// Check that the stream ended on a frame boundary.
    pub fn finish(&self) -> Result<(), FrameError> {
        if self.direct.is_none() && self.offset == self.buffer.len() {
            Ok(())
        } else {
            Err(FrameError::Truncated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Gate, PROTOCOL_VERSION};

    #[test]
    fn frames_round_trip_across_arbitrary_chunk_boundaries() {
        let hello = ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: "quest-1".into(),
            name: "Samuel".into(),
            names: false,
            environments: None,
            environment_state: false,
            projection: false,
            pairing_window: false,
            speech_held: false,
            images: false,
            projection_patches: false,
            live_panels: false,
            limits: None,
        };
        let audio = AudioFrame {
            gate: Gate::Agent,
            source: 0,
            sequence: 7,
            timestamp: 123_456,
            sample_rate: 16_000,
            samples: vec![1, -2, 3, i16::MIN, i16::MAX],
        };
        let mut bytes = Frame::client(&hello).encode();
        Frame::audio(&audio).encode_into(&mut bytes);

        for chunk in [1, 2, 3, 7, bytes.len()] {
            let mut decoder = FrameDecoder::new();
            let mut frames = Vec::new();
            for piece in bytes.chunks(chunk) {
                decoder.push(piece);
                while let Some(frame) = decoder.next_frame().unwrap() {
                    frames.push(frame);
                }
            }
            decoder.finish().unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!(frames[0].to_client_message().unwrap(), hello);
            assert_eq!(frames[1].to_audio().unwrap(), audio);
        }
    }

    #[test]
    fn rejects_unknown_kinds_oversized_frames_and_truncation() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&[9, 0, 0, 0, 0]);
        assert_eq!(
            decoder.next_frame().unwrap_err().to_string(),
            "unknown frame kind 9"
        );

        let mut decoder = FrameDecoder::new();
        decoder.push(&[0]);
        decoder.push(&((MAX_FRAME_BYTES as u32 + 1).to_le_bytes()));
        assert_eq!(
            decoder.next_frame().unwrap_err().to_string(),
            format!("frame of {} bytes exceeds the limit", MAX_FRAME_BYTES + 1)
        );

        let mut decoder = FrameDecoder::new();
        decoder.push(&[0, 4, 0, 0, 0, b'{']);
        assert!(decoder.next_frame().unwrap().is_none());
        assert_eq!(
            decoder.finish().unwrap_err().to_string(),
            "connection ended inside a frame"
        );
    }

    #[test]
    fn a_long_lived_decoder_discards_consumed_input() {
        let frame = Frame {
            kind: FrameKind::Audio,
            payload: vec![7; 1024],
        };
        let encoded = frame.encode();
        let mut decoder = FrameDecoder::new();
        // Each push leaves half a frame behind, so the buffer is never wholly consumed and is compacted only once the consumed prefix passes 64 KiB.
        let (first, second) = encoded.split_at(encoded.len() / 2);
        decoder.push(first);
        let mut decoded = 0;
        for _ in 0..100 {
            decoder.push(second);
            decoder.push(first);
            while let Some(next) = decoder.next_frame().unwrap() {
                assert_eq!(next, frame);
                decoded += 1;
            }
            assert!(decoder.offset <= 64 * 1024 + encoded.len());
            assert!(decoder.buffer.len() <= 64 * 1024 + 2 * encoded.len());
        }
        assert_eq!(decoded, 100);
        decoder.push(second);
        assert_eq!(decoder.next_frame().unwrap(), Some(frame));
        decoder.finish().unwrap();
    }

    #[test]
    fn oversized_payloads_are_never_encoded() {
        let frame = Frame {
            kind: FrameKind::Control,
            payload: vec![b' '; MAX_FRAME_BYTES + 1],
        };
        let mut output = vec![1, 2, 3];
        assert_eq!(
            frame.try_encode_into(&mut output).unwrap_err().to_string(),
            format!("frame of {} bytes exceeds the limit", MAX_FRAME_BYTES + 1)
        );
        assert_eq!(output, [1, 2, 3]);
        assert!(frame.try_encode().is_err());
        assert!(std::panic::catch_unwind(|| frame.encode()).is_err());

        let largest = Frame {
            kind: FrameKind::Control,
            payload: vec![b' '; MAX_FRAME_BYTES],
        };
        assert_eq!(
            largest.try_encode().unwrap().len(),
            HEADER_BYTES + MAX_FRAME_BYTES
        );
    }

    #[test]
    fn a_decoder_takes_frames_within_the_limit_negotiated_and_refuses_longer() {
        let header = |length: usize| {
            let mut bytes = vec![FrameKind::Control as u8];
            bytes.extend_from_slice(&(length as u32).to_le_bytes());
            bytes
        };
        let refused = |decoder: &mut FrameDecoder, length: usize| {
            decoder.push(&header(length));
            decoder.next_frame().unwrap_err().to_string()
                == format!("frame of {length} bytes exceeds the limit")
        };
        // Today's until both sides state more, as with an older peer.
        assert_eq!(FrameDecoder::new().limit(), MAX_FRAME_BYTES);
        assert!(refused(&mut FrameDecoder::new(), MAX_FRAME_BYTES + 1));
        // Both sides new.
        let mut decoder = FrameDecoder::new();
        decoder.set_limit(MOST_FRAME_BYTES);
        assert_eq!(decoder.limit(), MOST_FRAME_BYTES);
        decoder.push(&header(MAX_FRAME_BYTES + 1));
        assert!(decoder.next_frame().unwrap().is_none());
        assert!(refused(
            &mut FrameDecoder::with_limit(MOST_FRAME_BYTES),
            MOST_FRAME_BYTES + 1
        ));
        // However much is stated, no decoder takes more than the most any connection does, nor less than every peer.
        assert_eq!(
            FrameDecoder::with_limit(usize::MAX).limit(),
            MOST_FRAME_BYTES
        );
        assert_eq!(FrameDecoder::with_limit(16).limit(), MAX_FRAME_BYTES);
        assert_eq!(frame_limit(4 * MAX_FRAME_BYTES), 4 * MAX_FRAME_BYTES);
        // An encoder keeps within the connection's limit too, leaving the output as it was otherwise.
        let long = Frame {
            kind: FrameKind::Control,
            payload: vec![b' '; MAX_FRAME_BYTES + 1],
        };
        let mut output = vec![9];
        assert!(
            long.try_encode_within(&mut output, MAX_FRAME_BYTES)
                .is_err()
        );
        assert_eq!(output, [9]);
        long.try_encode_within(&mut output, MOST_FRAME_BYTES)
            .unwrap();
        assert_eq!(output.len(), 1 + HEADER_BYTES + MAX_FRAME_BYTES + 1);
    }

    #[test]
    fn a_long_frame_is_read_straight_into_its_payload_saying_how_much_has_arrived() {
        let message = ServerMessage::Error {
            error: "x".repeat(3 * MAX_FRAME_BYTES),
            space: None,
            panel: None,
            wall: None,
        };
        // Written straight into the output, after what is already there, as the server writes a projection.
        let mut bytes = vec![1, 0, 0, 0, 0];
        Frame::encode_control_within(&message, &mut bytes, MOST_FRAME_BYTES).unwrap();
        let mut whole = Vec::new();
        Frame::server(&message)
            .try_encode_within(&mut whole, MOST_FRAME_BYTES)
            .unwrap();
        assert_eq!(bytes[HEADER_BYTES..], whole[..]);
        let length = bytes.len() - 2 * HEADER_BYTES;
        // And another frame straight after it, in the same pieces.
        bytes.extend_from_slice(&[1, 0, 0, 0, 0]);
        // Too long for the connection, nothing is written.
        let mut short = vec![7];
        assert_eq!(
            Frame::encode_control_within(&message, &mut short, MAX_FRAME_BYTES)
                .unwrap_err()
                .to_string(),
            format!("frame of {length} bytes exceeds the limit")
        );
        assert_eq!(short, [7]);
        for chunk in [1000, DIRECT_BYTES + 1, 1 << 20] {
            let mut decoder = FrameDecoder::with_limit(MOST_FRAME_BYTES);
            let mut frames = Vec::new();
            let mut seen = Vec::new();
            for piece in bytes.chunks(chunk) {
                decoder.push(piece);
                while let Some(frame) = decoder.next_frame().unwrap() {
                    frames.push(frame);
                }
                if let Some(progress) = decoder.progress() {
                    assert_eq!(
                        (progress.kind, progress.length),
                        (FrameKind::Control, length)
                    );
                    seen.push(progress.received);
                }
                // Never held twice, nor more than declared.
                if let Some(direct) = &decoder.direct {
                    assert!(direct.payload.capacity() <= length);
                    assert!(decoder.buffer.len() - decoder.offset < chunk + HEADER_BYTES);
                }
            }
            decoder.finish().unwrap();
            assert_eq!(decoder.progress(), None);
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[2], frames[0]);
            assert_eq!(
                frames[0].to_audio().unwrap_err().to_string(),
                "invalid audio frame: header is truncated"
            );
            assert_eq!(frames[1].to_server_message().unwrap(), message);
            // How much has arrived rises to all but the last piece.
            assert!(seen.windows(2).all(|pair| pair[0] < pair[1]), "{seen:?}");
            assert!(seen.len() >= length / chunk - 1);
        }
        // Ending part way through a long frame is ending inside one.
        let mut decoder = FrameDecoder::with_limit(MOST_FRAME_BYTES);
        decoder.push(&bytes[HEADER_BYTES..HEADER_BYTES + DIRECT_BYTES * 2]);
        assert!(decoder.next_frame().unwrap().is_none());
        assert!(decoder.direct.is_some());
        assert_eq!(
            decoder.finish().unwrap_err().to_string(),
            "connection ended inside a frame"
        );
        // A short frame part way through is said to be arriving too, and its header alone is not.
        let mut decoder = FrameDecoder::new();
        decoder.push(&[0, 4, 0, 0]);
        assert_eq!(decoder.progress(), None);
        decoder.push(&[0, b'{']);
        assert_eq!(
            decoder.progress(),
            Some(Progress {
                kind: FrameKind::Control,
                received: 1,
                length: 4
            })
        );
        decoder.push(b"}  ");
        assert!(decoder.next_frame().unwrap().is_some());
        assert_eq!(decoder.progress(), None);
    }

    #[test]
    fn kind_mismatches_are_errors() {
        let frame = Frame::server(&ServerMessage::LEFT);
        assert_eq!(
            frame.to_audio().unwrap_err().to_string(),
            "unexpected Control frame"
        );
        let audio = Frame {
            kind: FrameKind::Audio,
            payload: Vec::new(),
        };
        assert_eq!(
            audio.to_server_message().unwrap_err().to_string(),
            "unexpected Audio frame"
        );
        assert_eq!(
            audio.to_audio().unwrap_err().to_string(),
            "invalid audio frame: header is truncated"
        );
        assert_eq!(
            audio.to_image().unwrap_err().to_string(),
            "unexpected Audio frame"
        );
        let image = Frame {
            kind: FrameKind::Image,
            payload: vec![0; 39],
        };
        assert_eq!(
            image.to_image().unwrap_err().to_string(),
            "invalid image frame: header is truncated"
        );
    }

    #[test]
    fn image_chunks_round_trip_in_frames_and_stay_within_their_image() {
        let hash = [7; 32];
        let image: Vec<u8> = (0..crate::IMAGE_CHUNK_BYTES * 2 + 5)
            .map(|index| index as u8)
            .collect();
        let chunks: Vec<ImageChunk> = ImageChunk::split(hash, &image).collect();
        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks.iter().map(|chunk| chunk.offset).collect::<Vec<_>>(),
            [
                0,
                crate::IMAGE_CHUNK_BYTES as u32,
                2 * crate::IMAGE_CHUNK_BYTES as u32
            ]
        );
        let mut bytes = Vec::new();
        for chunk in &chunks {
            assert!(chunk.data.len() <= crate::IMAGE_CHUNK_BYTES);
            Frame::image(chunk).encode_into(&mut bytes);
        }
        let mut decoder = FrameDecoder::new();
        decoder.push(&bytes);
        let mut joined = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            assert_eq!(frame.kind, FrameKind::Image);
            let chunk = frame.to_image().unwrap();
            assert_eq!((chunk.hash, chunk.total), (hash, image.len() as u32));
            assert_eq!(chunk.offset as usize, joined.len());
            joined.extend(chunk.data);
        }
        assert_eq!(joined, image);
        // A chunk reaching past its image, or of an image larger than any sent, is refused.
        let past = ImageChunk::new(hash, 4, 6, vec![1, 2, 3]).encode();
        assert_eq!(ImageChunk::decode(&past), Err(ImageChunkError::OutOfBounds));
        let huge = ImageChunk::new(hash, 0, crate::IMAGE_BYTES + 1, vec![1]).encode();
        assert_eq!(ImageChunk::decode(&huge), Err(ImageChunkError::OutOfBounds));
        let overflow = ImageChunk::new(hash, u32::MAX, u32::MAX, vec![1]).encode();
        assert_eq!(
            ImageChunk::decode(&overflow).unwrap_err().to_string(),
            "chunk is outside its image"
        );
        let last = ImageChunk::new(hash, 4, 6, vec![1, 2]);
        assert_eq!(ImageChunk::decode(&last.encode()), Ok(last));
    }

    #[test]
    fn malformed_control_messages_are_errors() {
        let frame = Frame {
            kind: FrameKind::Control,
            payload: b"{\"type\":\"no_such_message\"}".to_vec(),
        };
        let error = frame.to_client_message().unwrap_err().to_string();
        assert!(
            error.starts_with("invalid control message: unknown variant `no_such_message`"),
            "{error}"
        );
    }
}
