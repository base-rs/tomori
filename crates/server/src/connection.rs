// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base_http::body::{BodyError, BodyFrame, ReadableBody};
use base_scheduler::Cancellation;
use futures_util::FutureExt;
use futures_util::future::{Either, select};
use tomori_protocol::{Frame, FrameDecoder, ServerMessage};

use crate::access::Access;
use crate::hub::{ConnectionId, Hub, Output};
use crate::outbox::{Outbox, Popped};

/// Outgoing data is batched into chunks of about this size when frames are already waiting.
const CHUNK_TARGET: usize = 64 * 1024;

/// The most request reads applied before each outgoing chunk when both directions are busy.
const INPUT_READS_PER_OUTPUT: usize = 8;

/// The most request reads applied before the connection yields. Every connection, the keepalive sweep and the space drivers are polled by one task, and a request read completes without waiting while the peer's data is buffered, as does writing output while the socket has room. Without this, a peer that keeps sending faster than its input is applied, whether or not it reads what it is sent, would keep the rest of the server from running.
const INPUT_READS_PER_YIELD: usize = 64;

/// How many times in a row the connection returns pending when it yields. The HTTP server polls each read once and, if it is pending, flushes and then awaits the same read: when nothing is waiting to be written, that second poll comes at once, in the same poll of the task, and would complete a single yield without the task having yielded. The second pending reaches the server's set of connections, which returns to the task once two of its connections have yielded or it has polled each of them, so the keepalive sweep and the space drivers run after at most about twice `INPUT_READS_PER_YIELD + INPUT_READS_PER_OUTPUT` reads of one flooding connection. See `base_http1`'s `Output::write_body` and `FuturesUnordered`.
const YIELDS: usize = 2;

/// The response body of `POST /connect`: server frames out, while reading client frames from the request body it owns.
///
/// The HTTP server pulls this body to write the response. Each read returns the next outgoing frames, and while none are waiting it reads the request body and applies incoming frames to the [`Hub`]. No separate task is needed, so the connection's lifetime is exactly the HTTP exchange's. Dropping the body disconnects the participant.
pub struct ConnectionBody<B: ReadableBody> {
    hub: Arc<Hub>,
    id: ConnectionId,
    outbox: Arc<Outbox>,
    /// Set whenever request data arrives, for the hub's keepalive sweep.
    received: Arc<AtomicBool>,
    /// Tracks whether output is being written, for the hub's keepalive sweep.
    output: Arc<Output>,
    /// The request body. Its reads are raced against outgoing frames and dropped when those win, which the `ReadableBody` contract allows: a dropped read loses no input.
    request: B,
    decoder: FrameDecoder,
    /// Request reads applied since the connection last yielded; see [`INPUT_READS_PER_YIELD`].
    reads: usize,
    /// Pending returns still to make before reading again; see [`YIELDS`]. Kept here so that a read dropped while yielding leaves the rest for the next.
    yields: usize,
    input_ended: bool,
    finished: bool,
}

impl<B: ReadableBody> ConnectionBody<B> {
    /// `cancellation` must end only this connection's HTTP exchange; the hub cancels it when the connection goes silent. See [`Hub::sweep`]. Returns `None` if the hub refuses the connection, as a visitor beyond the limit.
    pub fn new(
        hub: Arc<Hub>,
        request: B,
        cancellation: Cancellation,
        access: Access,
    ) -> Option<Self> {
        let (id, outbox, received, output) = hub.connect(cancellation, access)?;
        Some(Self {
            hub,
            id,
            outbox,
            received,
            output,
            request,
            decoder: FrameDecoder::new(),
            reads: 0,
            yields: 0,
            input_ended: false,
            finished: false,
        })
    }

    /// Append waiting outgoing frames to `output`, up to the chunk target. Returns false once the outbox is closed and drained.
    fn drain(&mut self, output: &mut Vec<u8>) -> bool {
        while output.len() < CHUNK_TARGET {
            match self.outbox.pop() {
                Popped::Control(item) => {
                    if let Some(message) = self.hub.render(self.id, item) {
                        encode(&message, output);
                    }
                }
                Popped::Audio(frame) => output.extend_from_slice(&frame),
                Popped::Empty => break,
                Popped::Closed => return false,
            }
        }
        true
    }

    fn input<D: AsRef<[u8]>>(&mut self, frame: BodyFrame<D>) -> Result<(), BodyError> {
        self.reads += 1;
        let end = frame.is_end();
        if let BodyFrame::Data { data, .. } = &frame {
            // Any data counts as liveness, even part of a frame.
            self.received.store(true, Ordering::Release);
            self.decoder.push(data.as_ref());
            while let Some(frame) = self.decoder.next_frame().map_err(BodyError::new)? {
                self.hub.receive(self.id, frame);
            }
        }
        if end {
            self.decoder.finish().map_err(BodyError::new)?;
            self.input_ended = true;
        }
        Ok(())
    }

    /// Apply request data that has already arrived, without waiting. Output is only written when the HTTP server asks for it, so without this a connection whose outbox never empties would never read its input, and would look silent to the keepalive sweep.
    fn take_ready_input(&mut self, cancellation: &Cancellation) -> Result<(), BodyError> {
        // Bounded, so that a fast sender cannot starve output either.
        for _ in 0..INPUT_READS_PER_OUTPUT {
            if self.input_ended {
                break;
            }
            // A read that is not ready is dropped, losing nothing; the next real wait reads again with a real waker.
            let Some(result) = self.request.read(cancellation).now_or_never() else {
                break;
            };
            self.input(result.map_err(BodyError::new)?)?;
        }
        Ok(())
    }
}

/// Append `message` as a frame. A message that would exceed the frame limit, which the server's size limits should prevent, is replaced by a short error so that the client is not disconnected by a frame it must reject.
fn encode(message: &ServerMessage, output: &mut Vec<u8>) {
    if let Err(error) = Frame::server(message).try_encode_into(output) {
        log::error!("A server message was not sent: {error}.");
        Frame::server(&ServerMessage::Error {
            error: "A message from the server was too large to send.".to_owned(),
            space: None,
        })
        .encode_into(output);
    }
}

impl<B: ReadableBody> Drop for ConnectionBody<B> {
    fn drop(&mut self) {
        self.hub.disconnect(self.id);
    }
}

impl<B: ReadableBody> ReadableBody for ConnectionBody<B> {
    type Data = Vec<u8>;
    type Error = BodyError;

    fn is_ready(&self) -> bool {
        self.finished || self.outbox.is_ready()
    }

    fn may_have_trailers(&self) -> bool {
        false
    }

    fn close(&mut self, _error: Option<BodyError>) {
        self.finished = true;
        self.request.close(None);
    }

    async fn read(&mut self, cancellation: &Cancellation) -> Result<BodyFrame<Vec<u8>>, BodyError> {
        // The HTTP server reads the body again only once the previous chunk is written. Until then, the request body is not read either, so input from the peer is not seen while a write is blocked; the keepalive sweep releases a connection whose write makes no progress.
        self.output.asked();
        loop {
            if self.finished {
                return Ok(BodyFrame::End);
            }
            if self.reads >= INPUT_READS_PER_YIELD {
                self.reads = 0;
                self.yields = YIELDS;
            }
            while self.yields > 0 {
                self.yields -= 1;
                // Nothing has been read since the last input was applied, so dropping this read while it yields loses nothing.
                futures_lite::future::yield_now().await;
            }
            self.take_ready_input(cancellation)?;
            let mut output = Vec::new();
            let open = self.drain(&mut output);
            if !output.is_empty() {
                if !open {
                    self.finished = true;
                }
                self.output.handed_out();
                return Ok(BodyFrame::Data {
                    data: output,
                    end: !open,
                });
            }
            if !open || self.input_ended {
                // The client finished its upload, or the server ended the connection: end the response once drained.
                self.finished = true;
                return Ok(BodyFrame::End);
            }
            let listener = self.outbox.listen();
            if self.outbox.is_ready() {
                continue;
            }
            // When outgoing frames win, the pending read is dropped, losing no input; the next wait reads again from the same position.
            let received = match select(pin!(listener), pin!(self.request.read(cancellation))).await
            {
                Either::Left(_) => None,
                Either::Right((result, _)) => Some(result.map_err(BodyError::new)?),
            };
            if let Some(frame) = received {
                self.input(frame)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Poll, Waker};

    use tomori_protocol::ClientMessage;

    use super::*;
    use crate::transcriber::NullTranscriber;
    use crate::workspace::{DEFAULT_MAX_SPACES, SpaceSlots};
    use crate::{Access, Backends};

    /// More reads than a connection should ever apply without yielding. A flood stops being ready after this many, so that a connection that never yields fails the test rather than hanging it.
    const FLOOD_LIMIT: usize = 10_000;

    /// The most reads a connection applies before it yields: the limit is checked before each wait, and one wait reads at most `INPUT_READS_PER_OUTPUT + 1` times.
    const MOST_READS_BEFORE_YIELDING: usize = INPUT_READS_PER_YIELD + INPUT_READS_PER_OUTPUT;

    /// A request body whose every read completes at once with `message`, like a peer whose data is always buffered, for [`FLOOD_LIMIT`] reads.
    struct Flood {
        message: Vec<u8>,
        reads: Arc<AtomicUsize>,
    }

    impl ReadableBody for Flood {
        type Data = Vec<u8>;
        type Error = std::io::Error;

        fn close(&mut self, _error: Option<Self::Error>) {}

        async fn read(
            &mut self,
            _cancellation: &Cancellation,
        ) -> Result<BodyFrame<Vec<u8>>, Self::Error> {
            let reads = self.reads.fetch_add(1, Ordering::Relaxed);
            assert!(reads < FLOOD_LIMIT, "the connection never yielded");
            Ok(BodyFrame::Data {
                data: self.message.clone(),
                end: false,
            })
        }
    }

    /// A connection whose peer sends `message` without pause, and a count of the request reads.
    fn flooded(message: &ClientMessage) -> (ConnectionBody<Flood>, Arc<AtomicUsize>) {
        let (jobs, _) = async_channel::unbounded();
        let hub = Hub::new(
            Backends::new(),
            Arc::new(NullTranscriber::new()),
            jobs,
            Vec::new(),
            SpaceSlots::new(DEFAULT_MAX_SPACES),
        );
        let reads = Arc::new(AtomicUsize::new(0));
        let request = Flood {
            message: Frame::client(message).encode(),
            reads: reads.clone(),
        };
        let body = ConnectionBody::new(hub, request, Cancellation::new(), Access::open())
            .expect("an open server admits the connection");
        (body, reads)
    }

    /// Poll one read of `body` as `base_http1`'s `Output::write_body` does when nothing is waiting to be written: once, and if that is pending, at once again, in the same poll of its task. Returns pending only if the read would make the task yield.
    fn poll_read(body: &mut ConnectionBody<Flood>) -> Poll<Result<BodyFrame<Vec<u8>>, BodyError>> {
        let cancellation = Cancellation::new();
        let mut read = pin!(body.read(&cancellation));
        let mut context = Context::from_waker(Waker::noop());
        match read.as_mut().poll(&mut context) {
            Poll::Pending => read.as_mut().poll(&mut context),
            ready => ready,
        }
    }

    #[test]
    fn a_peer_whose_input_is_always_ready_and_needs_no_reply_lets_others_run() {
        // A pong queues no output, so without yielding the read would loop for as long as input is ready.
        let (mut body, reads) = flooded(&ClientMessage::Pong { nonce: 1 });
        assert!(poll_read(&mut body).is_pending());
        let applied = reads.load(Ordering::Relaxed);
        assert!(
            (INPUT_READS_PER_YIELD..=MOST_READS_BEFORE_YIELDING).contains(&applied),
            "{applied} reads before yielding"
        );
    }

    #[test]
    fn a_peer_whose_input_is_always_ready_and_answered_lets_others_run() {
        // Each ping queues a pong, so every read returns output, which the HTTP server may write without waiting; the connection still yields between reads.
        let (mut body, reads) = flooded(&ClientMessage::Ping { nonce: 1 });
        let mut outputs = 0;
        loop {
            let Poll::Ready(result) = poll_read(&mut body) else {
                break;
            };
            let Ok(BodyFrame::Data { .. }) = result else {
                unreachable!("each ping is answered with data, not {result:?}");
            };
            outputs += 1;
            assert!(outputs <= INPUT_READS_PER_YIELD, "never yielded");
        }
        assert!(outputs > 0);
        // Closing the response closes the request.
        body.close(None);
        assert!(body.finished);
        let applied = reads.load(Ordering::Relaxed);
        assert!(
            (INPUT_READS_PER_YIELD..=MOST_READS_BEFORE_YIELDING).contains(&applied),
            "{applied} reads before yielding"
        );
    }

    /// A request body that returns `frames` in turn, each after a wait if `waits`, then waits for ever.
    struct Script {
        frames: std::collections::VecDeque<Result<BodyFrame<Vec<u8>>, std::io::Error>>,
        waits: bool,
    }

    impl ReadableBody for Script {
        type Data = Vec<u8>;
        type Error = std::io::Error;

        fn close(&mut self, _error: Option<Self::Error>) {
            self.frames.clear();
        }

        async fn read(
            &mut self,
            _cancellation: &Cancellation,
        ) -> Result<BodyFrame<Vec<u8>>, Self::Error> {
            if self.waits {
                futures_lite::future::yield_now().await;
            }
            match self.frames.pop_front() {
                Some(frame) => frame,
                None => std::future::pending().await,
            }
        }
    }

    fn data(bytes: &[u8]) -> Result<BodyFrame<Vec<u8>>, std::io::Error> {
        Ok(BodyFrame::Data {
            data: bytes.to_vec(),
            end: false,
        })
    }

    fn scripted(
        frames: Vec<Result<BodyFrame<Vec<u8>>, std::io::Error>>,
        waits: bool,
    ) -> ConnectionBody<Script> {
        let (jobs, _) = async_channel::unbounded();
        let hub = Hub::new(
            Backends::new(),
            Arc::new(NullTranscriber::new()),
            jobs,
            Vec::new(),
            SpaceSlots::new(DEFAULT_MAX_SPACES),
        );
        let request = Script {
            frames: frames.into(),
            waits,
        };
        ConnectionBody::new(hub, request, Cancellation::new(), Access::open())
            .expect("an open server admits the connection")
    }

    fn read(body: &mut ConnectionBody<Script>) -> Result<BodyFrame<Vec<u8>>, BodyError> {
        futures_lite::future::block_on(body.read(&Cancellation::new()))
    }

    #[test]
    fn input_that_is_not_a_frame_or_ends_inside_one_fails_the_connection() {
        // A kind byte no frame has, read at once or after a wait.
        for waits in [false, true] {
            let mut body = scripted(vec![data(&[9, 0, 0, 0, 0])], waits);
            assert!(read(&mut body).is_err());
        }
        // The upload ends with half a frame.
        let mut body = scripted(
            vec![Ok(BodyFrame::Data {
                data: vec![0, 9],
                end: true,
            })],
            false,
        );
        assert!(read(&mut body).is_err());
        // The request cannot be read.
        let mut body = scripted(vec![Err(std::io::Error::other("reset"))], false);
        assert!(read(&mut body).is_err());
    }

    #[test]
    fn a_response_closed_ends_at_once_and_never_has_trailers() {
        let mut body = scripted(Vec::new(), false);
        assert!(!body.may_have_trailers());
        body.close(None);
        let Ok(BodyFrame::End) = read(&mut body) else {
            unreachable!("a closed response ends");
        };
    }

    #[test]
    fn a_space_rendered_for_a_connection_in_no_space_is_nothing() {
        let mut body = scripted(Vec::new(), false);
        body.outbox.space();
        let mut output = Vec::new();
        assert!(body.drain(&mut output));
        assert!(output.is_empty());
    }

    #[test]
    fn a_message_too_large_to_send_is_replaced_by_an_error_saying_so() {
        let mut output = Vec::new();
        encode(
            &ServerMessage::Error {
                error: "x".repeat(tomori_protocol::MAX_FRAME_BYTES),
                space: None,
            },
            &mut output,
        );
        let mut decoder = tomori_protocol::FrameDecoder::new();
        decoder.push(&output);
        let frame = decoder.next_frame().unwrap().unwrap();
        let Ok(ServerMessage::Error { error, .. }) = frame.to_server_message() else {
            unreachable!("the replacement is an error");
        };
        assert_eq!(error, "A message from the server was too large to send.");
    }
}
