// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection thread: one duplex exchange at a time, reconnecting with backoff.
use std::cell::Cell;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use base_http::body::{BodyFrame, ReadableBody};
use base_http::{Method, RequestMetadata};
use base_http_client::{Client as HttpClient, Endpoint, StreamingResponseBody};
use base_scheduler::{Cancellation, Clock, Timers};
use base_scheduler_native::NativeScheduler;
use futures_lite::future::{or, zip};
use tomori_protocol::{
    AudioFrame, CONNECT_PATH, CONTENT_TYPE, ClientMessage, Frame, FrameDecoder, FrameKind,
    Keepalive, PROTOCOL_VERSION, ServerMessage,
};
use tomori_security::{Fingerprint, Pinned};

use crate::client::Shared;
use crate::outbox::{Batch, Outbox};
use crate::resolve::resolve_in_background;
use crate::{ConnectionState, Disconnect, Event, Options, Transport};

/// The most encoded bytes written to the request body at once.
const BATCH_LIMIT: usize = 32 * 1024;

pub(crate) struct Context<'a> {
    pub scheduler: &'a NativeScheduler,
    pub shared: &'a Shared,
    pub options: &'a Options,
    pub audio: &'a SyncSender<AudioFrame>,
    /// The certificate of the first connection over TLS that the server welcomed, pinned for every connection after it, so that a reconnect never accepts another certificate, nor falls back to plain HTTP.
    pub learned: Cell<Option<Fingerprint>>,
}

impl Context<'_> {
    fn set_state(&self, state: ConnectionState) {
        self.shared.set_state(state);
    }

    /// Fail with [`Disconnect::Timeout`] if `future` does not finish within `duration`.
    async fn within<T>(
        &self,
        duration: Duration,
        future: impl Future<Output = Result<T, Disconnect>>,
    ) -> Result<T, Disconnect> {
        or(future, async {
            let _ = self.scheduler.sleep(duration, &Cancellation::never()).await;
            Err(Disconnect::Timeout)
        })
        .await
    }

    /// [`Disconnect::Closed`], once the client closes and `grace` has passed.
    async fn closed<T>(&self, grace: Duration) -> Result<T, Disconnect> {
        self.shared.closing.cancelled().await;
        let _ = self.scheduler.sleep(grace, &Cancellation::never()).await;
        Err(Disconnect::Closed)
    }
}

pub(crate) async fn run(context: &Context<'_>) {
    let mut failures = 0;
    while !context.shared.outbox.is_closed() {
        context.set_state(ConnectionState::Connecting {
            attempt: failures + 1,
        });
        let welcomed = Cell::new(None);
        let reason = session(context, &welcomed).await;
        let failed = context.shared.outbox.disconnect();
        context.shared.report(failed);
        if context.shared.outbox.is_closed() {
            break;
        }
        let welcomed_for = welcomed
            .get()
            .map(|since| context.scheduler.now().saturating_duration_since(since));
        failures = context
            .options
            .backoff
            .failures_after(failures, welcomed_for);
        let retry_in = context.options.backoff.delay(failures);
        context.set_state(ConnectionState::Disconnected { reason, retry_in });
        let _ = context
            .scheduler
            .sleep(retry_in, &context.shared.closing)
            .await;
    }
    context.set_state(ConnectionState::Closed);
}

fn transport(error: impl std::fmt::Display) -> Disconnect {
    Disconnect::Transport(error.to_string())
}

/// Run one connection until it ends, returning why. Resolution and connecting share the connect timeout; each resolved address is tried in turn until one accepts the connection.
async fn session(context: &Context<'_>, welcomed: &Cell<Option<Instant>>) -> Disconnect {
    let deadline = context.scheduler.now() + context.options.connect_timeout;
    // Closing does not wait for a slow lookup.
    let resolution = or(
        context.within(
            context.options.connect_timeout,
            resolve_in_background(&context.options.address),
        ),
        context.closed(Duration::ZERO),
    );
    let addresses = match resolution.await {
        Ok(addresses) => addresses,
        Err(reason) => return reason,
    };
    let mut reason = Disconnect::Timeout;
    for address in addresses {
        let opened = Cell::new(false);
        reason = connection(context, address, deadline, &opened, welcomed).await;
        // Only an address that refused the connection is skipped: once the request is open, queued messages may have been taken for it.
        let refused = !opened.get() && matches!(reason, Disconnect::Transport(_));
        if !refused || context.shared.outbox.is_closed() || context.scheduler.now() >= deadline {
            break;
        }
    }
    reason
}

/// Run one connection to `address` until it ends, returning why, over the transport the options choose. `opened` is set once the request is open.
async fn connection(
    context: &Context<'_>,
    address: SocketAddr,
    deadline: Instant,
    opened: &Cell<bool>,
    welcomed: &Cell<Option<Instant>>,
) -> Disconnect {
    match effective(context.options.transport, context.learned.get()) {
        Transport::Plain => attempt(context, address, None, deadline, opened, welcomed).await,
        Transport::Tls { pin } => {
            let pinned = Pinned::new(pin);
            attempt(context, address, Some(pinned), deadline, opened, welcomed).await
        }
        Transport::Automatic { pin } => {
            let pinned = Pinned::new(pin);
            let reason = attempt(
                context,
                address,
                Some(pinned.clone()),
                deadline,
                opened,
                welcomed,
            )
            .await;
            // A server that presented no certificate does not speak TLS, as in `--auth open` mode: try plain HTTP, which presents no token.
            if speaks_plain(&reason, opened.get(), pinned.seen())
                && context.scheduler.now() < deadline
            {
                return attempt(context, address, None, deadline, opened, welcomed).await;
            }
            reason
        }
    }
}

/// The transport for the next connection: the options' choice, pinning the certificate `learned` from an earlier connection if the options pin none.
fn effective(transport: Transport, learned: Option<Fingerprint>) -> Transport {
    match transport {
        Transport::Plain => Transport::Plain,
        // Once a certificate is known, whether given or learned, the server speaks TLS: never fall back to plain HTTP, so that a changed certificate is reported.
        Transport::Automatic { pin: None } if learned.is_none() => {
            Transport::Automatic { pin: None }
        }
        Transport::Tls { pin } | Transport::Automatic { pin } => Transport::Tls {
            pin: pin.or(learned),
        },
    }
}

/// Whether an automatic connection should try plain HTTP after TLS failed with `reason`: the connection failed before its request opened, and no certificate was presented.
fn speaks_plain(reason: &Disconnect, opened: bool, seen: Option<Fingerprint>) -> bool {
    !opened && seen.is_none() && matches!(reason, Disconnect::Transport(_))
}

/// The `Authorization` field value for `token`, which is presented only over TLS.
fn authorization(token: Option<&str>, tls: bool) -> Option<String> {
    token.filter(|_| tls).map(|token| format!("Bearer {token}"))
}

/// Why a TLS connection failed before its request opened: a certificate other than the pinned one, or the transport `error`.
fn refused(pinned: Option<&Pinned>, error: Disconnect) -> Disconnect {
    match (
        pinned.and_then(Pinned::seen),
        pinned.and_then(Pinned::expected),
    ) {
        (Some(seen), Some(expected)) if seen != expected => {
            Disconnect::Certificate(seen.to_string())
        }
        _ => error,
    }
}

/// One connection to `address`, over TLS if `pinned` is given, which decides the certificate accepted.
async fn attempt(
    context: &Context<'_>,
    address: SocketAddr,
    pinned: Option<Arc<Pinned>>,
    deadline: Instant,
    opened: &Cell<bool>,
    welcomed: &Cell<Option<Instant>>,
) -> Disconnect {
    let endpoint = match &pinned {
        // The certificate is pinned rather than checked against its name, so the name is only for the handshake.
        Some(pinned) => Endpoint::new(address)
            .with_tls(tomori_security::client_config(pinned.clone()), "tomori"),
        None => Endpoint::new(address),
    };
    let (client, driver) = HttpClient::new(context.scheduler, endpoint, Default::default())
        .expect("the default HTTP client options are valid");
    let lifetime = Cancellation::new();
    let exchange = async {
        let exchange = exchange(
            context,
            &client,
            &lifetime,
            deadline,
            opened,
            welcomed,
            pinned.as_deref(),
        );
        // Closing lets the exchange deliver queued control messages, but only for so long.
        let ended: Result<Infallible, Disconnect> = or(
            async { Err(exchange.await) },
            context.closed(context.options.close_timeout),
        )
        .await;
        let Err(reason) = ended;
        client.begin_close();
        lifetime.cancel();
        reason
    };
    let (reason, _) = zip(exchange, driver.run(&lifetime)).await;
    reason
}

async fn exchange(
    context: &Context<'_>,
    client: &HttpClient<'_>,
    lifetime: &Cancellation,
    deadline: Instant,
    opened: &Cell<bool>,
    welcomed: &Cell<Option<Instant>>,
    pinned: Option<&Pinned>,
) -> Disconnect {
    let mut metadata = RequestMetadata::new(Method::POST);
    if let Some(authorization) = authorization(context.options.token.as_deref(), pinned.is_some()) {
        // A token that is not a valid field value is not presented, and the server admits the connection as it would without one.
        let _ = metadata.fields.set("authorization", authorization);
    }
    if context.options.pairing {
        metadata
            .fields
            .set("tomori-pairing", "1")
            .expect("a valid field value");
    }
    metadata.path = Some(CONNECT_PATH.into());
    metadata
        .fields
        .set("content-type", CONTENT_TYPE)
        .expect("the content type is a valid field value");
    metadata
        .fields
        .set("accept", CONTENT_TYPE)
        .expect("the content type is a valid field value");
    let remaining = deadline.saturating_duration_since(context.scheduler.now());
    let request = context
        .within(remaining, async {
            client
                .open_request(metadata, lifetime)
                .await
                .map_err(transport)
        })
        .await;
    let (mut output, response) = match request {
        Ok(request) => request.split(),
        Err(reason) => return refused(pinned, reason),
    };
    let certificate = pinned.and_then(Pinned::seen);
    opened.set(true);
    // Wait for the server's answer before sending anything. A server that refuses the connection, such as with 401, closes it after answering, and data it has not read would reset the connection and lose the answer. The server sends its response head at once, before any frame.
    let response = context
        .within(context.options.connect_timeout, async {
            response.await.map_err(transport)
        })
        .await;
    let mut response = match response {
        Ok(response) if response.status.is_success() => response,
        Ok(response) => return Disconnect::Status(response.status.as_u16()),
        Err(reason) => return reason,
    };

    let upload = async {
        let options = context.options;
        let hello = ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: options.participant.clone(),
            name: context
                .shared
                .name
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            // Every client built with this crate reads `Name`, which it hands on as any other message.
            names: true,
            environments: options.environments.clone(),
            // Every client built with this crate reads environments' state and events, which it hands on as any other message, for the client side of the environment, if it has one, to present.
            environment_state: true,
            // Every client built with this crate reads what is projected in its space, which it hands on as any other message. See `design/projection.md`.
            projection: true,
        };
        let mut bytes = Frame::client(&hello).encode();
        for message in context.shared.outbox.connect() {
            Frame::client(&message).encode_into(&mut bytes);
        }
        // A write fails only as the connection does, and its response with it, so the download says why. Writing stops then; otherwise the request body is finished once the client closes, with queued control sent, and the server then ends its response, which `session` bounds by the close timeout.
        let mut written = output.write(bytes, lifetime).await;
        while written.is_ok()
            && let Batch::Frames(bytes) = context.shared.outbox.next_batch(BATCH_LIMIT).await
        {
            written = output.write(bytes, lifetime).await;
        }
        let _ = output.finish(lifetime).await;
    };

    let download = async {
        let result: Result<Infallible, Disconnect> = async {
            let received = Cell::new(context.scheduler.now());
            let mut reader = FrameReader {
                body: &mut response.body,
                decoder: FrameDecoder::new(),
                ended: false,
                lifetime,
                scheduler: context.scheduler,
                received: &received,
            };
            let first = context
                .within(context.options.connect_timeout, reader.next())
                .await?;
            let source = welcome(context, first, certificate)?;
            // The first certificate seen is the one learned.
            context.learned.set(context.learned.get().or(certificate));
            welcomed.set(Some(context.scheduler.now()));
            let frames = async {
                loop {
                    let frame = reader.next().await?;
                    deliver(context, frame, source)?;
                }
            };
            let keepalive = keepalive(
                context.scheduler,
                context.options.keepalive,
                &context.shared.outbox,
                &received,
            );
            or(frames, async { Err(keepalive.await) }).await
        }
        .await;
        match result {
            Err(reason) => reason,
        }
    };

    // The response says how the connection ended.
    alongside(download, upload).await
}

/// `main`, with `background` making progress beside it until it is done.
async fn alongside<T>(main: impl Future<Output = T>, background: impl Future<Output = ()>) -> T {
    let mut main = std::pin::pin!(main);
    let mut background = std::pin::pin!(background);
    let mut done = false;
    std::future::poll_fn(|context| {
        done = done || background.as_mut().poll(context).is_ready();
        main.as_mut().poll(context)
    })
    .await
}

/// Ping the server, through `outbox`, whenever nothing has been received for the keepalive interval, and end the connection with [`Disconnect::Timeout`] once nothing has been received for the keepalive timeout. `received` is when data last arrived.
async fn keepalive<S: Timers>(
    scheduler: &S,
    keepalive: Keepalive,
    outbox: &Outbox,
    received: &Cell<Instant>,
) -> Disconnect {
    let mut timer = scheduler.timer();
    let mut nonce = 0;
    let mut pinged: Option<Instant> = None;
    loop {
        let last = received.get();
        // Ping once the link has been quiet for an interval, and again each interval after that.
        let ping_at = match pinged {
            Some(pinged) if pinged >= last => pinged + keepalive.interval,
            _ => last + keepalive.interval,
        };
        let deadline = ping_at.min(last + keepalive.timeout);
        // Data arriving meanwhile does not wake this; the next wake simply finds a later deadline.
        if let Err(error) = scheduler
            .timer_wait_until(&mut timer, deadline, &Cancellation::never())
            .await
        {
            return transport(error);
        }
        let now = scheduler.now();
        let last = received.get();
        if now >= last + keepalive.timeout {
            return Disconnect::Timeout;
        }
        if now >= ping_at && now >= last + keepalive.interval {
            outbox.ping(nonce);
            nonce = nonce.wrapping_add(1);
            pinged = Some(now);
        }
    }
}

/// Check the server's Welcome, returning this client's participant number. `certificate` is the server's, over TLS.
fn welcome(
    context: &Context<'_>,
    frame: Frame,
    certificate: Option<Fingerprint>,
) -> Result<u32, Disconnect> {
    let message = frame
        .to_server_message()
        .map_err(|error| Disconnect::Protocol(error.to_string()))?;
    match message {
        ServerMessage::Welcome {
            protocol,
            server,
            source,
            role,
            backends,
            roots,
            environments,
            utterance_limit,
            name,
        } => {
            if protocol != PROTOCOL_VERSION {
                return Err(Disconnect::Protocol(format!(
                    "server speaks protocol {protocol}, client speaks {PROTOCOL_VERSION}"
                )));
            }
            context.set_state(ConnectionState::Connected {
                source,
                server,
                role,
                certificate,
                backends,
                roots,
                environments,
                utterance_limit: utterance_limit.map(|seconds| Duration::from_secs(seconds.into())),
                name,
            });
            Ok(source)
        }
        ServerMessage::Error { error, .. } => Err(Disconnect::Protocol(format!(
            "server refused the connection: {error}"
        ))),
        other => Err(Disconnect::Protocol(format!(
            "expected welcome, received {other:?}"
        ))),
    }
}

/// Hand one frame to the application. `source` is this client's participant number on this connection.
fn deliver(context: &Context<'_>, frame: Frame, source: u32) -> Result<(), Disconnect> {
    let protocol = |error: tomori_protocol::FrameError| Disconnect::Protocol(error.to_string());
    match frame.kind {
        FrameKind::Control => match frame.to_server_message().map_err(protocol)? {
            ServerMessage::Welcome { .. } => {
                return Err(Disconnect::Protocol("unexpected second welcome".into()));
            }
            // Keepalive is handled here and not reported: receiving it already counted as liveness.
            ServerMessage::Ping { nonce } => context.shared.outbox.pong(nonce),
            ServerMessage::Pong { .. } => {}
            message => {
                match &message {
                    ServerMessage::Space { state } => {
                        context.shared.outbox.joined(Some(state.id.clone()))
                    }
                    ServerMessage::Left => context.shared.outbox.joined(None),
                    // Suggested from now on, as the server calls this participant.
                    ServerMessage::Name { name } => {
                        name.clone_into(
                            &mut context
                                .shared
                                .name
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner),
                        );
                    }
                    _ => {}
                }
                context.shared.events.push(Event::Message(message));
            }
        },
        FrameKind::Audio => {
            let audio = frame.to_audio().map_err(protocol)?;
            // What this participant records never plays back to them. The server does not relay it back; this guards against a server that does.
            if audio.source == source {
                return Ok(());
            }
            // Playback is real time: if the application falls behind, newer audio is dropped rather than queued without bound.
            let _ = context.audio.try_send(audio);
        }
    }
    Ok(())
}

struct FrameReader<'a, 'scope> {
    body: &'a mut StreamingResponseBody<'scope>,
    decoder: FrameDecoder,
    ended: bool,
    lifetime: &'a Cancellation,
    scheduler: &'a NativeScheduler,
    /// When data last arrived: any data counts as liveness, even part of a frame.
    received: &'a Cell<Instant>,
}

impl FrameReader<'_, '_> {
    /// The next complete frame, or why the response ended.
    async fn next(&mut self) -> Result<Frame, Disconnect> {
        let protocol = |error: tomori_protocol::FrameError| Disconnect::Protocol(error.to_string());
        loop {
            if let Some(frame) = self.decoder.next_frame().map_err(protocol)? {
                return Ok(frame);
            }
            if self.ended {
                self.decoder.finish().map_err(protocol)?;
                return Err(Disconnect::Closed);
            }
            self.ended = match self.body.read(self.lifetime).await.map_err(transport)? {
                BodyFrame::Data { data, end } => {
                    self.received.set(self.scheduler.now());
                    self.decoder.push(data.as_ref());
                    end
                }
                BodyFrame::Headers { end, .. } => end,
                BodyFrame::End => true,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Timers whose waits fail.
    struct Broken;

    impl Clock for Broken {
        fn now(&self) -> Instant {
            Instant::now()
        }

        fn sleep_until(
            &self,
            _deadline: Instant,
            _cancellation: &Cancellation,
        ) -> impl Future<Output = base_scheduler::OperationResult<()>> + Send {
            std::future::ready(Err(std::io::Error::other("the timer broke").into()))
        }
    }

    impl Timers for Broken {
        type Timer = ();

        fn timer(&self) {}

        fn timer_wait_until(
            &self,
            _timer: &mut (),
            deadline: Instant,
            cancellation: &Cancellation,
        ) -> impl Future<Output = base_scheduler::OperationResult<()>> + Send {
            self.sleep_until(deadline, cancellation)
        }
    }

    #[test]
    fn a_keepalive_whose_timer_fails_ends_the_connection() {
        let outbox = Outbox::new(1, 1);
        let received = Cell::new(Broken.now());
        let reason = futures_lite::future::block_on(keepalive(
            &Broken,
            Keepalive::default(),
            &outbox,
            &received,
        ));
        assert_eq!(reason, Disconnect::Transport("the timer broke".into()));
    }

    #[test]
    fn plain_http_is_tried_only_when_no_certificate_was_presented() {
        let failed = Disconnect::Transport("handshake".into());
        let seen = Some(Fingerprint::of(b"certificate"));
        assert!(speaks_plain(&failed, false, None));
        assert!(!speaks_plain(&failed, false, seen));
        assert!(!speaks_plain(&failed, true, None));
        assert!(!speaks_plain(&Disconnect::Timeout, false, None));
    }

    #[test]
    fn debugging_options_never_shows_secrets() {
        let mut options = Options::new("studio:8765", "device-secret", "Ada");
        options.token = Some("tomori_secret".into());
        let debugged = format!("{options:?}");
        assert!(!debugged.contains("tomori_secret"), "{debugged}");
        assert!(!debugged.contains("device-secret"), "{debugged}");
        assert!(debugged.contains("studio:8765"));
    }

    #[test]
    fn the_token_is_presented_only_over_tls() {
        assert_eq!(
            authorization(Some("tomori_abc"), true),
            Some("Bearer tomori_abc".to_owned())
        );
        assert_eq!(authorization(Some("tomori_abc"), false), None);
        assert_eq!(authorization(None, true), None);
    }

    #[test]
    fn a_different_certificate_is_reported_as_changed() {
        let ours = Fingerprint::of(b"ours");
        let pinned = Pinned::new(Some(ours));
        let error = || Disconnect::Transport("handshake".into());
        // Nothing presented yet: the transport error stands.
        assert_eq!(refused(Some(&pinned), error()), error());
        use rustls::client::danger::ServerCertVerifier;
        let _ = pinned.verify_server_cert(
            &rustls::pki_types::CertificateDer::from(b"theirs".to_vec()),
            &[],
            &rustls::pki_types::ServerName::try_from("tomori").unwrap(),
            &[],
            rustls::pki_types::UnixTime::now(),
        );
        let theirs = Fingerprint::of(b"theirs").to_string();
        assert_eq!(
            refused(Some(&pinned), error()),
            Disconnect::Certificate(theirs)
        );
        assert_eq!(refused(None, error()), error());
        // Without a pin, any certificate is accepted, so a failure is the transport's.
        assert_eq!(refused(Some(&Pinned::new(None)), error()), error());
    }

    #[test]
    fn a_learned_certificate_is_pinned_from_then_on() {
        let learned = Some(Fingerprint::of(b"learned"));
        let given = Some(Fingerprint::of(b"given"));
        assert_eq!(effective(Transport::Plain, learned), Transport::Plain);
        assert_eq!(
            effective(Transport::Automatic { pin: None }, None),
            Transport::Automatic { pin: None }
        );
        // Once TLS has worked, an automatic connection no longer falls back to plain HTTP.
        assert_eq!(
            effective(Transport::Automatic { pin: None }, learned),
            Transport::Tls { pin: learned }
        );
        assert_eq!(
            effective(Transport::Tls { pin: None }, learned),
            Transport::Tls { pin: learned }
        );
        assert_eq!(
            effective(Transport::Tls { pin: given }, learned),
            Transport::Tls { pin: given }
        );
        // An automatic connection given a pin speaks TLS from the start.
        assert_eq!(
            effective(Transport::Automatic { pin: given }, None),
            Transport::Tls { pin: given }
        );
    }
}
