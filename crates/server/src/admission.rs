// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Deadlines for HTTP connections that are not serving a request.
//!
//! A peer that opens a socket and sends nothing, or trickles a request head, or leaves a keep-alive connection idle, would otherwise hold one of the server's connection slots for ever. Each connection is idle from when it is accepted until its request head is complete, and again from when a response is finished until the next head is complete. A connection that stays idle for the request timeout is cancelled. `/connect` exchanges are then covered by the hub, which releases a connection that has not said Hello within the same timeout, and by the keepalive sweep. See `design/server.md`.
//!
//! TODO(upstream): base-http1's `RequestReader::read_request` and base-http-server's `serve_connection` have no request-head or keep-alive idle timeout. Once `base_http1::Options` (or `base_http_server::Options`) offers a deadline for reading a request head, measured from accept and from the end of the previous response, use it and remove this module.
use std::convert::Infallible;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use base_http::body::{BodyFrame, ReadableBody};
use base_http::{InterimHandler, InterimResponses, Method, Request, Response};
use base_http_server::Handler;
use base_scheduler::Cancellation;

use crate::application::Application;

/// The default request timeout.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The connections being watched.
#[derive(Default)]
pub struct Admission {
    watches: Mutex<Vec<Weak<Watch>>>,
}

/// One connection's progress.
pub struct Watch {
    /// When the connection became idle, as first seen by a sweep, or the request being served.
    idle: Mutex<Idle>,
    /// Ends only this connection.
    cancellation: Cancellation,
}

enum Idle {
    /// Idle, and not yet seen by a sweep.
    Unseen,
    Since(Instant),
    /// Serving a request, named by its method and the start of its path.
    Serving(String),
}

/// The most characters of a request path kept to name the request in logs.
const LOGGED_PATH: usize = 64;

impl Watch {
    fn set(&self, idle: Idle) {
        *self.idle.lock().expect("the watch is never left poisoned") = idle;
    }

    /// What the connection was last seen doing, for logs.
    pub fn state(&self) -> String {
        match &*self.idle.lock().expect("the watch is never left poisoned") {
            Idle::Unseen | Idle::Since(_) => {
                "waiting for a request or closing after a response".to_owned()
            }
            Idle::Serving(request) => format!("serving {request}"),
        }
    }
}

impl Admission {
    /// Watch a newly accepted connection, which is idle until its first request head is complete.
    pub fn watch(&self, cancellation: Cancellation) -> Arc<Watch> {
        let watch = Arc::new(Watch {
            idle: Mutex::new(Idle::Unseen),
            cancellation,
        });
        let mut watches = self
            .watches
            .lock()
            .expect("the watches are never left poisoned");
        watches.retain(|watch| watch.strong_count() > 0);
        watches.push(Arc::downgrade(&watch));
        watch
    }

    /// Cancel connections that have been idle for `timeout`. An idle connection's time is measured from the first sweep that sees it idle.
    pub fn sweep(&self, now: Instant, timeout: Duration) {
        let mut expired = Vec::new();
        {
            let mut watches = self
                .watches
                .lock()
                .expect("the watches are never left poisoned");
            watches.retain(|watch| {
                let Some(watch) = watch.upgrade() else {
                    return false;
                };
                let mut idle = watch.idle.lock().expect("the watch is never left poisoned");
                match *idle {
                    Idle::Unseen => *idle = Idle::Since(now),
                    Idle::Since(since) if now.saturating_duration_since(since) >= timeout => {
                        expired.push(watch.cancellation.clone());
                        return false;
                    }
                    Idle::Since(_) | Idle::Serving(_) => {}
                }
                true
            });
        }
        if !expired.is_empty() {
            log::debug!(
                "Closing {} connections that sent no request in time.",
                expired.len()
            );
        }
        for cancellation in expired {
            cancellation.cancel();
        }
    }
}

/// The application, or another handler such as Tomori's MCP endpoint, serving one watched connection.
pub struct Admitted<'a, H = Application> {
    pub application: &'a H,
    pub watch: Arc<Watch>,
}

impl<H: Handler<Error = Infallible>> Handler for Admitted<'_, H> {
    type Error = Infallible;
    type Body<B: ReadableBody> = Tracked<H::Body<B>>;

    async fn call<B: ReadableBody, I: InterimHandler>(
        &self,
        request: Request<B>,
        interim: &mut InterimResponses<I>,
        cancellation: &Cancellation,
    ) -> Result<Response<Self::Body<B>>, Infallible> {
        // The request head is complete.
        let path = request.path.as_ref().map_or("", |path| path.as_str());
        let path = path.split('?').next().unwrap_or_default();
        // A posted form's body is read in the handler, so the request stays under the request timeout until it is answered: a peer that trickles a form cannot hold the connection. Only `/connect` streams for as long as it lasts, under the keepalive sweep instead.
        if request.method == Method::POST && path != tomori_protocol::CONNECT_PATH {
            self.watch.set(Idle::Unseen);
        } else {
            let path: String = path.chars().take(LOGGED_PATH).collect();
            self.watch
                .set(Idle::Serving(format!("{} {path}", request.method)));
        }
        let Ok(response) = self.application.call(request, interim, cancellation).await;
        let watch = self.watch.clone();
        Ok(response.map_body(|body| Tracked { body, watch }))
    }
}

/// A response body that marks its connection idle once the response is finished.
pub struct Tracked<B> {
    body: B,
    watch: Arc<Watch>,
}

impl<B> Drop for Tracked<B> {
    fn drop(&mut self) {
        self.watch.set(Idle::Unseen);
    }
}

impl<B: ReadableBody> ReadableBody for Tracked<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn is_ready(&self) -> bool {
        self.body.is_ready()
    }

    fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    fn length(&self) -> Option<u64> {
        self.body.length()
    }

    fn may_have_trailers(&self) -> bool {
        self.body.may_have_trailers()
    }

    fn close(&mut self, error: Option<B::Error>) {
        self.body.close(error)
    }

    async fn read(&mut self, cancellation: &Cancellation) -> Result<BodyFrame<B::Data>, B::Error> {
        self.body.read(cancellation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty request body.
    struct Nothing;

    impl ReadableBody for Nothing {
        type Data = Vec<u8>;
        type Error = std::io::Error;

        fn is_empty(&self) -> bool {
            true
        }

        fn close(&mut self, _error: Option<Self::Error>) {}

        async fn read(&mut self, _: &Cancellation) -> Result<BodyFrame<Vec<u8>>, Self::Error> {
            Ok(BodyFrame::End)
        }
    }

    #[test]
    fn a_connection_says_what_it_was_last_seen_doing() {
        let admission = Admission::default();
        let watch = admission.watch(Cancellation::new());
        assert_eq!(
            watch.state(),
            "waiting for a request or closing after a response"
        );
        watch.set(Idle::Serving("GET /".into()));
        assert_eq!(watch.state(), "serving GET /");
    }

    #[test]
    fn a_connection_that_sends_no_request_in_time_is_closed() {
        log::set_max_level(log::LevelFilter::Trace);
        let admission = Admission::default();
        let cancellation = Cancellation::new();
        let _watch = admission.watch(cancellation.clone());
        let now = Instant::now();
        let timeout = Duration::from_secs(10);
        admission.sweep(now, timeout);
        assert!(!cancellation.is_cancelled());
        admission.sweep(now + timeout, timeout);
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn a_tracked_body_is_as_empty_as_its_body() {
        let watch = Admission::default().watch(Cancellation::new());
        let tracked = Tracked {
            body: Nothing,
            watch,
        };
        assert!(tracked.is_empty());
        let mut nothing = Nothing;
        assert!(futures_lite::future::block_on(nothing.read(&Cancellation::new())).is_ok());
        nothing.close(None);
    }
}
