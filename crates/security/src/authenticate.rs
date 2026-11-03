// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// The cookie in which a browser holds its token, set by the server's sign-in page. The `__Host-` prefix makes browsers keep it only as set: secure, for the whole server and no other host.
pub const TOKEN_COOKIE: &str = "__Host-tomori-token";

/// The credentials presented on the HTTP request that opens a connection or asks for a page. See `design/security.md`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    /// The bearer token from the `Authorization` header, or else from the [`TOKEN_COOKIE`] cookie.
    pub token: Option<String>,
}

/// Written without the token, so that logging credentials cannot reveal it.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

impl Credentials {
    /// Read the credentials from the request's `Authorization` and `Cookie` field values. An `Authorization` field that is not a bearer token is presented as an empty token, so that it is refused rather than ignored.
    pub fn from_fields<'a>(
        authorization: Option<&str>,
        cookies: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        if let Some(authorization) = authorization {
            let token = authorization
                .split_once(' ')
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map(|(_, token)| token.trim().to_owned())
                .unwrap_or_default();
            return Self { token: Some(token) };
        }
        let token = cookies
            .into_iter()
            .flat_map(|field| field.split(';'))
            .filter_map(|cookie| cookie.trim().split_once('='))
            .find(|(name, _)| *name == TOKEN_COOKIE)
            .map(|(_, value)| value.trim().to_owned());
        Self { token }
    }
}

/// Who a connection is, as an authenticator verified it: an opaque identifier, namespaced by that authenticator, such as `token:…`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Principal {
    pub id: String,
    /// A name to show for the principal, such as on the status page.
    pub name: String,
}

/// One authenticator's view of a set of credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The credentials identify this principal.
    Authenticated(Principal),
    /// Not this authenticator's credentials; ask the next.
    Pass,
    /// This authenticator's credentials, but not acceptable, such as a malformed token.
    Refuse,
}

/// Says who presented a set of credentials. Never decides what they may do.
pub trait Authenticator: Send + Sync {
    fn authenticate(&self, credentials: &Credentials) -> Verdict;
}

/// What a [`Chain`] concluded about a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No credentials were presented.
    Anonymous,
    Authenticated(Principal),
    /// Credentials were presented and no authenticator accepted them. The server refuses the request with the same reply whatever the reason.
    Refused,
}

/// Authenticators tried in order: the first to recognize the credentials decides.
#[derive(Default)]
pub struct Chain {
    authenticators: Vec<Box<dyn Authenticator>>,
}

impl Chain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, authenticator: impl Authenticator + 'static) -> Self {
        self.authenticators.push(Box::new(authenticator));
        self
    }

    pub fn authenticate(&self, credentials: &Credentials) -> Outcome {
        if credentials.token.is_none() {
            return Outcome::Anonymous;
        }
        for authenticator in &self.authenticators {
            match authenticator.authenticate(credentials) {
                Verdict::Authenticated(principal) => return Outcome::Authenticated(principal),
                Verdict::Refuse => return Outcome::Refused,
                Verdict::Pass => {}
            }
        }
        Outcome::Refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(token: &str) -> Credentials {
        Credentials {
            token: Some(token.to_owned()),
        }
    }

    #[test]
    fn reads_a_bearer_token_before_the_cookie() {
        let cookies = ["theme=dark; __Host-tomori-token=from-cookie"];
        assert_eq!(
            Credentials::from_fields(Some("Bearer from-header"), cookies),
            token("from-header")
        );
        assert_eq!(
            Credentials::from_fields(Some("bearer  spaced "), []),
            token("spaced")
        );
        assert_eq!(
            Credentials::from_fields(None, cookies),
            token("from-cookie")
        );
        assert_eq!(
            Credentials::from_fields(None, ["a=1", "__Host-tomori-token=second"]),
            token("second")
        );
        assert_eq!(
            Credentials::from_fields(None, ["theme=dark", "broken"]),
            Credentials::default()
        );
    }

    #[test]
    fn debugging_never_shows_the_token() {
        let debugged = format!("{:?}", token("tomori_secret"));
        assert!(!debugged.contains("tomori_secret"), "{debugged}");
        assert!(debugged.contains("[redacted]"));
    }

    #[test]
    fn an_authorization_that_is_not_a_bearer_token_is_presented_empty() {
        assert_eq!(
            Credentials::from_fields(Some("Basic dXNlcg=="), ["__Host-tomori-token=cookie"]),
            token("")
        );
        assert_eq!(Credentials::from_fields(Some("Bearer"), []), token(""));
    }

    struct Fixed(&'static str, Verdict);

    impl Authenticator for Fixed {
        fn authenticate(&self, credentials: &Credentials) -> Verdict {
            if credentials.token.as_deref() == Some(self.0) {
                self.1.clone()
            } else {
                Verdict::Pass
            }
        }
    }

    #[test]
    fn the_first_authenticator_to_recognize_the_credentials_decides() {
        let ada = Principal {
            id: "test:ada".into(),
            name: "Ada".into(),
        };
        let chain = Chain::new()
            .with(Fixed("bad", Verdict::Refuse))
            .with(Fixed("ada", Verdict::Authenticated(ada.clone())))
            .with(Fixed("bad", Verdict::Authenticated(ada.clone())));
        assert_eq!(
            chain.authenticate(&Credentials::default()),
            Outcome::Anonymous
        );
        assert_eq!(
            chain.authenticate(&token("ada")),
            Outcome::Authenticated(ada)
        );
        assert_eq!(chain.authenticate(&token("bad")), Outcome::Refused);
        assert_eq!(chain.authenticate(&token("unknown")), Outcome::Refused);
        assert_eq!(Chain::new().authenticate(&token("")), Outcome::Refused);
    }
}
