// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::convert::Infallible;
use std::sync::Arc;

use base_http::body::{BodyError, BodyFrame, ReadableBody};
use base_http::{Fields, InterimHandler, InterimResponses, Method, Request, Response, Status};
use base_http_server::Handler;
use base_scheduler::Cancellation;
use tomori_security::Credentials;

use crate::access::Grant;
use crate::connection::ConnectionBody;
use crate::door::Door;
use crate::hub::Hub;
use crate::pages;

/// The HTTP routes of the server: `POST /connect` for client connections and `GET /health`.
///
/// The cancellation passed to each call must be specific to that HTTP connection, as [`crate::Server::run`] arranges: a connection that goes silent is ended by cancelling it.
pub struct Application {
    hub: Arc<Hub>,
    door: Door,
}

impl Application {
    pub(crate) fn new(hub: Arc<Hub>) -> Self {
        Self {
            hub,
            door: Door::Open,
        }
    }

    pub(crate) fn set_door(&mut self, door: Door) {
        self.door = door;
    }

    pub(crate) fn door(&self) -> &Door {
        &self.door
    }
}

/// A web page, with headers that keep scripts, frames and other origins out of it.
fn html<B: ReadableBody>(status: u16, page: String) -> Response<ResponseBody<B>> {
    let mut response = response(
        Status::new(status).expect("valid status"),
        "text/html; charset=utf-8",
        ResponseBody::fixed(page),
    );
    response
        .fields
        .add("Content-Security-Policy", pages::CONTENT_SECURITY_POLICY)
        .expect("valid policy");
    response
        .fields
        .add("X-Content-Type-Options", "nosniff")
        .expect("valid option");
    response
}

/// Send the browser to `location`, setting `cookie` if given.
fn redirect<B: ReadableBody>(location: &str, cookie: Option<String>) -> Response<ResponseBody<B>> {
    let mut response = response(
        Status::new(303).expect("valid status"),
        "text/plain",
        ResponseBody::fixed(""),
    );
    response
        .fields
        .add("Location", location)
        .expect("valid location");
    if let Some(cookie) = cookie {
        // A cookie holding a token has only token characters, which are valid field syntax.
        response
            .fields
            .add("Set-Cookie", cookie)
            .expect("valid cookie");
    }
    response
}

/// The cookie that holds a browser's token, or clears it. See `design/security.md`.
fn token_cookie(token: Option<&str>) -> String {
    let attributes = "Path=/; Secure; HttpOnly; SameSite=Strict";
    match token {
        Some(token) => format!("{}={token}; {attributes}", tomori_security::TOKEN_COOKIE),
        None => format!(
            "{}=; {attributes}; Max-Age=0",
            tomori_security::TOKEN_COOKIE
        ),
    }
}

/// Whether a form was posted from this server's own pages: its `Origin`, when given, names the host the request was sent to, its `authority`. The cookie's `SameSite=Strict` already keeps other sites' forms from carrying it.
fn same_origin(fields: &Fields, authority: Option<&str>) -> bool {
    let origin = fields
        .values("origin")
        .next()
        .map(|value| std::str::from_utf8(value).unwrap_or_default());
    match origin {
        None => true,
        Some(origin) => {
            let host = origin.split_once("://").map_or(origin, |(_, host)| host);
            authority.is_some_and(|authority| authority.eq_ignore_ascii_case(host))
        }
    }
}

/// Read a form body of at most [`pages::FORM_LIMIT`] bytes.
async fn read_form<B: ReadableBody>(mut body: B, cancellation: &Cancellation) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        match body.read(cancellation).await.ok()? {
            BodyFrame::Data { data, end } => {
                bytes.extend_from_slice(data.as_ref());
                if bytes.len() > pages::FORM_LIMIT {
                    return None;
                }
                if end {
                    return Some(bytes);
                }
            }
            // Trailers follow all the data, so the form is whole.
            BodyFrame::Headers { .. } | BodyFrame::End => return Some(bytes),
        }
    }
}

/// Who a page request is from.
enum Viewer {
    /// An owner, as everyone is in `--auth open` mode, by the name of their token, if they presented one.
    Owner(Option<String>),
    /// Signed in, but not as an owner.
    Other,
    /// Not signed in, or with a token that is no longer accepted.
    Nobody,
}

impl Application {
    fn viewer(&self, fields: &Fields) -> Viewer {
        let credentials = credentials(fields);
        match self.door.admit(&credentials, false) {
            Some(access) if access.grant == Grant::Owner => Viewer::Owner(access.name),
            Some(_) if credentials.token.is_some() => Viewer::Other,
            _ => Viewer::Nobody,
        }
    }

    /// Whether the request carries a token cookie, so that signing out is offered.
    fn signed_in(fields: &Fields) -> bool {
        credentials(fields).token.is_some()
    }

    fn status_page<B: ReadableBody>(&self, fields: &Fields) -> Response<ResponseBody<B>> {
        match self.viewer(fields) {
            Viewer::Owner(_) => {
                let tokens = self.door.store().map(|store| store.entries());
                html(
                    200,
                    pages::status(
                        &self.hub.status(),
                        self.hub.pairing_state().as_ref(),
                        self.hub.pending_pairing().as_ref(),
                        self.hub.pairing_paused(),
                        tokens.as_deref(),
                        Self::signed_in(fields),
                    ),
                )
            }
            Viewer::Other => html(403, pages::not_an_owner()),
            Viewer::Nobody => redirect(pages::SIGN_IN_PATH, None),
        }
    }

    /// Answer a form posted to `path`. The form has been read first, whatever the answer, since a request body left unread would reset the connection and lose the answer.
    fn form<B: ReadableBody>(
        &self,
        path: &str,
        fields: &Fields,
        authority: Option<&str>,
        form: Option<Vec<u8>>,
    ) -> Response<ResponseBody<B>> {
        if !same_origin(fields, authority) {
            return html(
                403,
                pages::sign_in(Some("That form came from another site.")),
            );
        }
        match path {
            pages::SIGN_IN_PATH => self.sign_in(form.as_deref()),
            pages::SIGN_OUT_PATH => redirect(pages::SIGN_IN_PATH, Some(token_cookie(None))),
            pages::ALLOW_PAIRING_PATH => {
                if !matches!(self.viewer(fields), Viewer::Owner(_)) {
                    return html(403, pages::not_an_owner());
                }
                self.hub.allow_pairing();
                log::info!("Allowed pairing again from the status page.");
                redirect(pages::STATUS_PATH, None)
            }
            pages::CONFIRM_PAIRING_PATH | pages::DECLINE_PAIRING_PATH => {
                self.decide_pairing(fields, form.as_deref(), path == pages::CONFIRM_PAIRING_PATH)
            }
            pages::OPEN_PAIRING_PATH | pages::CLOSE_PAIRING_PATH => {
                self.open_pairing(fields, path == pages::OPEN_PAIRING_PATH)
            }
            _ => self.revoke(fields, form.as_deref()),
        }
    }

    /// Confirm or decline the pairing the status page showed, as an owner who compared the codes. A different pairing pending now is left alone.
    fn decide_pairing<B: ReadableBody>(
        &self,
        fields: &Fields,
        form: Option<&[u8]>,
        confirm: bool,
    ) -> Response<ResponseBody<B>> {
        if !matches!(self.viewer(fields), Viewer::Owner(_)) {
            return html(403, pages::not_an_owner());
        }
        let Some(serial) = form
            .and_then(|form| pages::form_value(form, "pairing"))
            .and_then(|serial| serial.parse::<u64>().ok())
        else {
            return redirect(pages::STATUS_PATH, None);
        };
        if confirm {
            match self
                .hub
                .confirm_pairing(crate::hub::Confirmation::Shown(serial))
            {
                Ok(device) => log::info!("Confirmed pairing {device} from the status page."),
                Err(error) => log::info!("Could not confirm a pairing: {error}"),
            }
        } else {
            self.hub.decline_pairing(Some(serial));
        }
        redirect(pages::STATUS_PATH, None)
    }

    /// Open pairing for a new device, or close it, as an owner on the status page. See `design/security.md`.
    fn open_pairing<B: ReadableBody>(
        &self,
        fields: &Fields,
        open: bool,
    ) -> Response<ResponseBody<B>> {
        let Viewer::Owner(name) = self.viewer(fields) else {
            return html(403, pages::not_an_owner());
        };
        if open {
            if let Err(error) = self
                .hub
                .open_pairing(crate::hub::Opener::status_page(name.as_deref()))
            {
                log::info!("Could not open pairing from the status page: {error}");
            }
        } else {
            self.hub.close_pairing(format!(
                "{} closed it on the status page.",
                name.as_deref().unwrap_or("An owner")
            ));
        }
        redirect(pages::STATUS_PATH, None)
    }

    fn sign_in<B: ReadableBody>(&self, form: Option<&[u8]>) -> Response<ResponseBody<B>> {
        let token = form
            .and_then(|form| pages::form_value(form, "token"))
            .map(|token| token.trim().to_owned());
        let credentials = Credentials {
            token: token.clone(),
        };
        match (token, self.door.admit(&credentials, false)) {
            (Some(token), Some(access)) if access.principal.is_some() => {
                redirect(pages::STATUS_PATH, Some(token_cookie(Some(&token))))
            }
            _ => html(401, pages::sign_in(Some("That token is not accepted."))),
        }
    }

    fn revoke<B: ReadableBody>(
        &self,
        fields: &Fields,
        form: Option<&[u8]>,
    ) -> Response<ResponseBody<B>> {
        if !matches!(self.viewer(fields), Viewer::Owner(_)) {
            return html(403, pages::not_an_owner());
        }
        let id = form.and_then(|form| pages::form_value(form, "id"));
        if let (Some(store), Some(id)) = (self.door.store(), id)
            && crate::door::revoke(store, &id)
        {
            log::info!("Revoked token {id} from the status page.");
            self.hub
                .revoke(|principal| store.entry(principal).is_some());
        }
        redirect(pages::STATUS_PATH, None)
    }
}

/// The credentials on a request. An `Authorization` field that is not text is presented as an empty token, so that it is refused.
fn credentials(fields: &Fields) -> Credentials {
    fn text(value: &[u8]) -> Option<&str> {
        std::str::from_utf8(value).ok()
    }
    let authorization = fields
        .values("authorization")
        .next()
        .map(|value| text(value).unwrap_or_default());
    Credentials::from_fields(authorization, fields.values("cookie").filter_map(text))
}

/// A response body: a client connection, a small fixed document, or content by digest.
pub enum ResponseBody<B: ReadableBody> {
    Connection(ConnectionBody<B>),
    Fixed(Option<Vec<u8>>, u64),
    Content(ContentBody),
}

/// The bytes of content a response sends, from one offset to another, in pieces of at most
/// [`crate::content::PIECE_BYTES`], holding its connection's permit to have the request open until it is dropped.
pub struct ContentBody {
    bytes: Arc<[u8]>,
    at: usize,
    end: usize,
    _permit: crate::content::Permit,
}

impl<B: ReadableBody> ResponseBody<B> {
    pub(crate) fn fixed(data: impl Into<Vec<u8>>) -> Self {
        let data = data.into();
        let length = data.len() as u64;
        Self::Fixed(Some(data), length)
    }
}

impl<B: ReadableBody> ReadableBody for ResponseBody<B> {
    type Data = Vec<u8>;
    type Error = BodyError;

    fn is_ready(&self) -> bool {
        match self {
            Self::Connection(body) => body.is_ready(),
            Self::Fixed(..) | Self::Content(_) => true,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Content(body) => body.at >= body.end,
            _ => matches!(self, Self::Fixed(None, _)),
        }
    }

    fn length(&self) -> Option<u64> {
        match self {
            Self::Connection(_) => None,
            Self::Fixed(_, length) => Some(*length),
            Self::Content(body) => Some((body.end - body.at) as u64),
        }
    }

    fn may_have_trailers(&self) -> bool {
        false
    }

    fn close(&mut self, error: Option<BodyError>) {
        match self {
            Self::Connection(body) => body.close(error),
            Self::Fixed(data, _) => *data = None,
            Self::Content(body) => body.at = body.end,
        }
    }

    async fn read(&mut self, cancellation: &Cancellation) -> Result<BodyFrame<Vec<u8>>, BodyError> {
        match self {
            Self::Connection(body) => body.read(cancellation).await,
            Self::Fixed(data, _) => Ok(match data.take() {
                Some(data) => BodyFrame::Data { data, end: true },
                None => BodyFrame::End,
            }),
            Self::Content(body) => Ok(if body.at >= body.end {
                BodyFrame::End
            } else {
                let until = body.end.min(body.at + crate::content::PIECE_BYTES);
                let data = body.bytes[body.at..until].to_vec();
                body.at = until;
                BodyFrame::Data {
                    data,
                    end: until == body.end,
                }
            }),
        }
    }
}

pub(crate) fn response<B: ReadableBody>(
    status: Status,
    content_type: &str,
    body: ResponseBody<B>,
) -> Response<ResponseBody<B>> {
    let mut response = Response::new(status, body);
    // These names and values are valid header syntax.
    response
        .fields
        .add("Content-Type", content_type)
        .expect("valid content type");
    response
        .fields
        .add("Cache-Control", "no-store")
        .expect("valid cache control");
    response
}

impl Handler for Application {
    type Error = Infallible;
    type Body<B: ReadableBody> = ResponseBody<B>;

    async fn call<B: ReadableBody, I: InterimHandler>(
        &self,
        request: Request<B>,
        _interim: &mut InterimResponses<I>,
        cancellation: &Cancellation,
    ) -> Result<Response<ResponseBody<B>>, Infallible> {
        let path = request.path.as_ref().map_or("", |path| path.as_str());
        let path = path.split('?').next().unwrap_or_default();
        let get = request.method == Method::GET;
        let head = request.method == Method::HEAD;
        let post = request.method == Method::POST;
        Ok(match path {
            tomori_protocol::CONNECT_PATH if post => {
                // Credentials are checked before the connection reaches the hub; every refusal gets the same reply. See `design/security.md`.
                // A device without credentials may ask to pair on a server that refuses visitors; see `design/security.md`.
                let to_pair = request
                    .fields
                    .values("tomori-pairing")
                    .next()
                    .is_some_and(|value| value == b"1");
                let Some(access) = self.door.admit(&credentials(&request.fields), to_pair) else {
                    log::debug!("Refused a connection whose credentials were not accepted.");
                    let mut refused = response(
                        Status::new(401).expect("valid status"),
                        "text/plain",
                        ResponseBody::fixed("Unauthorized.\n"),
                    );
                    refused
                        .fields
                        .add("WWW-Authenticate", "Bearer")
                        .expect("valid challenge");
                    return Ok(refused);
                };
                match ConnectionBody::new(
                    self.hub.clone(),
                    request.body,
                    cancellation.clone(),
                    access,
                ) {
                    Some(body) => response(
                        Status::OK,
                        tomori_protocol::CONTENT_TYPE,
                        ResponseBody::Connection(body),
                    ),
                    None => response(
                        Status::new(503).expect("valid status"),
                        "text/plain",
                        ResponseBody::fixed("Too many visitors are connected.\n"),
                    ),
                }
            }
            content if (get || head) && content.starts_with(tomori_protocol::CONTENT_PATH) => self
                .content(
                    &request,
                    &content[tomori_protocol::CONTENT_PATH.len()..],
                    head,
                ),
            pages::STATUS_PATH if get => self.status_page(&request.fields),
            pages::SIGN_IN_PATH if get => match self.door {
                Door::Open => redirect(pages::STATUS_PATH, None),
                Door::Tokens { .. } => html(200, pages::sign_in(None)),
            },
            pages::SIGN_IN_PATH
            | pages::SIGN_OUT_PATH
            | pages::REVOKE_PATH
            | pages::CONFIRM_PAIRING_PATH
            | pages::DECLINE_PAIRING_PATH
            | pages::ALLOW_PAIRING_PATH
            | pages::OPEN_PAIRING_PATH
            | pages::CLOSE_PAIRING_PATH
                if post =>
            {
                let path = path.to_owned();
                let fields = request.fields.clone();
                let authority = request
                    .authority
                    .as_ref()
                    .map(|authority| authority.as_str().to_owned());
                let form = read_form(request.body, cancellation).await;
                self.form(&path, &fields, authority.as_deref(), form)
            }
            "/health" if get => response(
                Status::OK,
                "application/json",
                ResponseBody::fixed(format!(
                    "{{\"status\":\"ok\",\"server\":\"{}\",\"protocol\":{}}}\n",
                    crate::SERVER_NAME,
                    tomori_protocol::PROTOCOL_VERSION
                )),
            ),
            tomori_protocol::CONNECT_PATH
            | "/health"
            | pages::STATUS_PATH
            | pages::SIGN_IN_PATH
            | pages::SIGN_OUT_PATH
            | pages::REVOKE_PATH
            | pages::CONFIRM_PAIRING_PATH
            | pages::DECLINE_PAIRING_PATH
            | pages::ALLOW_PAIRING_PATH
            | pages::OPEN_PAIRING_PATH
            | pages::CLOSE_PAIRING_PATH => response(
                Status::new(405).expect("valid status"),
                "text/plain",
                ResponseBody::fixed("Method not allowed.\n"),
            ),
            content if content.starts_with(tomori_protocol::CONTENT_PATH) => response(
                Status::new(405).expect("valid status"),
                "text/plain",
                ResponseBody::fixed("Method not allowed.\n"),
            ),
            _ => not_found(),
        })
    }
}

/// What a request for anything not served is answered: content not kept and content not offered alike.
fn not_found<B: ReadableBody>() -> Response<ResponseBody<B>> {
    response(
        Status::NOT_FOUND,
        "text/plain",
        ResponseBody::fixed("Not found.\n"),
    )
}

/// The refusal of a request whose credentials, or session ticket, are not accepted: the same for every reason.
fn unauthorized<B: ReadableBody>() -> Response<ResponseBody<B>> {
    let mut refused = response(
        Status::new(401).expect("valid status"),
        "text/plain",
        ResponseBody::fixed("Unauthorized.\n"),
    );
    refused
        .fields
        .add("WWW-Authenticate", "Bearer")
        .expect("valid challenge");
    refused
}

/// The answer to a content request refused for `refusal`.
fn refused<B: ReadableBody>(refusal: crate::content::Refusal) -> Response<ResponseBody<B>> {
    use crate::content::Refusal;
    match refusal {
        Refusal::Unbound => {
            log::debug!("Refused a content request whose ticket names no connection of its own.");
            unauthorized()
        }
        Refusal::Missing => not_found(),
        Refusal::Busy => response(
            Status::new(503).expect("valid status"),
            "text/plain",
            ResponseBody::fixed("Too many content requests are open.\n"),
        ),
    }
}

/// The fields every content response carries: it is immutable, named by its digest, kept by no shared cache, never
/// sniffed or run, and read by no other origin. See `design/content.md`.
fn content_fields(fields: &mut Fields, etag: &str) {
    for (name, value) in [
        ("ETag", etag),
        ("Cache-Control", "private, max-age=31536000, immutable"),
        ("Accept-Ranges", "bytes"),
        ("X-Content-Type-Options", "nosniff"),
        ("Content-Security-Policy", "default-src 'none'; sandbox"),
        ("Cross-Origin-Resource-Policy", "same-origin"),
    ] {
        fields.add(name, value).expect("valid content fields");
    }
}

impl Application {
    /// Answer `GET` or `HEAD /content/HEX`: the content whose SHA-256 is `hex`, in lower case, if the request's
    /// credentials and session ticket name a connection it is offered to, whole or one range of it. See
    /// `design/content.md`.
    fn content<B: ReadableBody>(
        &self,
        request: &Request<B>,
        hex: &str,
        head: bool,
    ) -> Response<ResponseBody<B>> {
        use crate::content::Range;
        let Some(digest) =
            tomori_assets::Digest::parse(hex).filter(|digest| digest.to_string() == hex)
        else {
            return not_found();
        };
        if !request.body.is_empty() {
            return response(
                Status::new(400).expect("valid status"),
                "text/plain",
                ResponseBody::fixed("A content request has no body.\n"),
            );
        }
        let Some(access) = self.door.admit(&credentials(&request.fields), false) else {
            log::debug!("Refused a content request whose credentials were not accepted.");
            return unauthorized();
        };
        let field = |name| {
            request
                .fields
                .values(name)
                .next()
                .and_then(|value| std::str::from_utf8(value).ok())
        };
        let Some(ticket) = field(tomori_protocol::SESSION_FIELD) else {
            return unauthorized();
        };
        let served = match self.hub.content(&access, ticket, &digest) {
            Ok(served) => served,
            Err(refusal) => return refused(refusal),
        };
        let etag = format!("\"{hex}\"");
        let length = served.bytes.len() as u64;
        // A client that keeps it already is told so, as for any tag it lists, or for any at all.
        let kept = request.fields.values("if-none-match").any(|value| {
            std::str::from_utf8(value).is_ok_and(|value| {
                value
                    .split(',')
                    .any(|tag| matches!(tag.trim(), "*") || tag.trim() == etag)
            })
        });
        let range = if kept {
            Range::Whole
        } else {
            Range::of(field("range"), length)
        };
        let (status, start, end) = match range {
            _ if kept => (304, 0, 0),
            Range::Whole => (200, 0, length),
            Range::Part(start, end) => (206, start, end),
            Range::Unsatisfiable => (416, 0, 0),
        };
        let body = if head || matches!(status, 304 | 416) {
            ResponseBody::Fixed(None, 0)
        } else {
            ResponseBody::Content(ContentBody {
                bytes: served.bytes,
                at: start as usize,
                end: end as usize,
                _permit: served.permit,
            })
        };
        let mut answer = Response::new(Status::new(status).expect("valid status"), body);
        if status != 416 {
            answer
                .fields
                .add("Content-Type", served.kind.content_type())
                .expect("valid content type");
            content_fields(&mut answer.fields, &etag);
        }
        match status {
            206 => answer
                .fields
                .add(
                    "Content-Range",
                    format!("bytes {start}-{}/{length}", end - 1),
                )
                .expect("valid range"),
            416 => answer
                .fields
                .add("Content-Range", format!("bytes */{length}"))
                .expect("valid range"),
            _ => {}
        }
        answer.metadata.content_length = (status != 304).then_some(end - start);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request body with nothing in it.
    struct Nothing;

    impl ReadableBody for Nothing {
        type Data = Vec<u8>;
        type Error = std::io::Error;

        fn close(&mut self, _error: Option<Self::Error>) {}

        async fn read(&mut self, _: &Cancellation) -> Result<BodyFrame<Vec<u8>>, Self::Error> {
            Ok(BodyFrame::End)
        }
    }

    #[test]
    fn a_fixed_body_is_read_once_and_is_empty_once_read() {
        let mut body = ResponseBody::<Nothing>::fixed("ok");
        assert!(!body.is_empty());
        let read = |body: &mut ResponseBody<Nothing>| {
            futures_lite::future::block_on(body.read(&Cancellation::new())).unwrap()
        };
        let BodyFrame::Data { data, end: true } = read(&mut body) else {
            unreachable!("a fixed body is one frame");
        };
        assert_eq!(data, b"ok");
        assert!(body.is_empty());
        let BodyFrame::End = read(&mut body) else {
            unreachable!("a fixed body read again has ended");
        };
        let mut nothing = Nothing;
        assert!(futures_lite::future::block_on(nothing.read(&Cancellation::new())).is_ok());
        nothing.close(None);
    }

    #[test]
    fn content_is_handed_out_in_pieces_until_its_end_and_ends_once_closed() {
        let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let bytes: Arc<[u8]> = (0..crate::content::PIECE_BYTES * 2 + 10)
            .map(|index| index as u8)
            .collect();
        let body = |at, end| {
            ResponseBody::<Nothing>::Content(ContentBody {
                bytes: bytes.clone(),
                at,
                end,
                _permit: crate::content::Permit::take(&open).unwrap(),
            })
        };
        let read = |body: &mut ResponseBody<Nothing>| {
            futures_lite::future::block_on(body.read(&Cancellation::new())).unwrap()
        };
        let mut whole = body(5, bytes.len());
        assert!(whole.is_ready() && !whole.is_empty() && !whole.may_have_trailers());
        assert_eq!(whole.length(), Some(bytes.len() as u64 - 5));
        let mut received = Vec::new();
        loop {
            match read(&mut whole) {
                BodyFrame::Data { data, end } => {
                    assert!(data.len() <= crate::content::PIECE_BYTES);
                    received.extend(data);
                    if end {
                        break;
                    }
                }
                _ => unreachable!("data comes until its end"),
            }
        }
        assert_eq!(received, &bytes[5..]);
        assert!(whole.is_empty());
        assert!(matches!(read(&mut whole), BodyFrame::End));
        let mut closed = body(0, 10);
        closed.close(None);
        assert!(closed.is_empty());
        assert!(matches!(read(&mut closed), BodyFrame::End));
        // Each holds its connection's permit until it is dropped.
        assert_eq!(open.load(std::sync::atomic::Ordering::Acquire), 2);
        drop((whole, closed));
        assert_eq!(open.load(std::sync::atomic::Ordering::Acquire), 0);
    }

    #[test]
    fn a_content_request_refused_is_answered_as_its_refusal_says() {
        use crate::content::Refusal;
        let status = |refusal| refused::<Nothing>(refusal).status.as_u16();
        assert_eq!(
            [Refusal::Unbound, Refusal::Missing, Refusal::Busy].map(status),
            [401, 404, 503]
        );
    }
}
