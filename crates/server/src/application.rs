// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::convert::Infallible;
use std::sync::Arc;

use base_http::body::{BodyError, BodyFrame, ReadableBody};
use base_http::{Fields, InterimHandler, InterimResponses, Method, Request, Response, Status};
use base_http_server::Handler;
use base_scheduler::Cancellation;
use security::Credentials;

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
        Some(token) => format!("{}={token}; {attributes}", security::TOKEN_COOKIE),
        None => format!("{}=; {attributes}; Max-Age=0", security::TOKEN_COOKIE),
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
            BodyFrame::Headers { end: false, .. } => {}
            BodyFrame::Headers { end: true, .. } | BodyFrame::End => return Some(bytes),
        }
    }
}

/// Who a page request is from.
enum Viewer {
    /// An owner, as everyone is in `--auth open` mode.
    Owner,
    /// Signed in, but not as an owner.
    Other,
    /// Not signed in, or with a token that is no longer accepted.
    Nobody,
}

impl Application {
    fn viewer(&self, fields: &Fields) -> Viewer {
        let credentials = credentials(fields);
        match self.door.admit(&credentials) {
            Some(access) if access.grant == Grant::Owner => Viewer::Owner,
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
            Viewer::Owner => {
                let tokens = self.door.store().map(|store| store.entries());
                html(
                    200,
                    pages::status(
                        &self.hub.status(),
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
            _ => self.revoke(fields, form.as_deref()),
        }
    }

    fn sign_in<B: ReadableBody>(&self, form: Option<&[u8]>) -> Response<ResponseBody<B>> {
        let token = form
            .and_then(|form| pages::form_value(form, "token"))
            .map(|token| token.trim().to_owned());
        let credentials = Credentials {
            token: token.clone(),
        };
        match (token, self.door.admit(&credentials)) {
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
        if !matches!(self.viewer(fields), Viewer::Owner) {
            return html(403, pages::not_an_owner());
        }
        let id = form.and_then(|form| pages::form_value(form, "id"));
        if let (Some(store), Some(id)) = (self.door.store(), id) {
            match store.revoke(&id) {
                Ok(true) => {
                    log::info!("Revoked token {id} from the status page.");
                    self.hub
                        .revoke(|principal| store.entry(principal).is_some());
                }
                Ok(false) => {}
                Err(error) => log::warn!("Could not revoke token {id}: {error}."),
            }
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

/// A response body: either a client connection or a small fixed document.
pub enum ResponseBody<B: ReadableBody> {
    Connection(ConnectionBody<B>),
    Fixed(Option<Vec<u8>>, u64),
}

impl<B: ReadableBody> ResponseBody<B> {
    fn fixed(data: impl Into<Vec<u8>>) -> Self {
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
            Self::Fixed(..) => true,
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Fixed(None, _))
    }

    fn length(&self) -> Option<u64> {
        match self {
            Self::Connection(_) => None,
            Self::Fixed(_, length) => Some(*length),
        }
    }

    fn may_have_trailers(&self) -> bool {
        false
    }

    fn close(&mut self, error: Option<BodyError>) {
        match self {
            Self::Connection(body) => body.close(error),
            Self::Fixed(data, _) => *data = None,
        }
    }

    async fn read(&mut self, cancellation: &Cancellation) -> Result<BodyFrame<Vec<u8>>, BodyError> {
        match self {
            Self::Connection(body) => body.read(cancellation).await,
            Self::Fixed(data, _) => Ok(match data.take() {
                Some(data) => BodyFrame::Data { data, end: true },
                None => BodyFrame::End,
            }),
        }
    }
}

fn response<B: ReadableBody>(
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
        let post = request.method == Method::POST;
        Ok(match path {
            protocol::CONNECT_PATH if post => {
                // Credentials are checked before the connection reaches the hub; every refusal gets the same reply. See `design/security.md`.
                let Some(access) = self.door.admit(&credentials(&request.fields)) else {
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
                        protocol::CONTENT_TYPE,
                        ResponseBody::Connection(body),
                    ),
                    None => response(
                        Status::new(503).expect("valid status"),
                        "text/plain",
                        ResponseBody::fixed("Too many visitors are connected.\n"),
                    ),
                }
            }
            pages::STATUS_PATH if get => self.status_page(&request.fields),
            pages::SIGN_IN_PATH if get => match self.door {
                Door::Open => redirect(pages::STATUS_PATH, None),
                Door::Tokens { .. } => html(200, pages::sign_in(None)),
            },
            pages::SIGN_IN_PATH | pages::SIGN_OUT_PATH | pages::REVOKE_PATH if post => {
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
                    protocol::PROTOCOL_VERSION
                )),
            ),
            protocol::CONNECT_PATH
            | "/health"
            | pages::STATUS_PATH
            | pages::SIGN_IN_PATH
            | pages::SIGN_OUT_PATH
            | pages::REVOKE_PATH => response(
                Status::new(405).expect("valid status"),
                "text/plain",
                ResponseBody::fixed("Method not allowed.\n"),
            ),
            _ => response(
                Status::NOT_FOUND,
                "text/plain",
                ResponseBody::fixed("Not found.\n"),
            ),
        })
    }
}
