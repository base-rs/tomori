// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Bearer tokens, and the sources a client asks for them.
use std::fmt;

/// A bearer token, sent only in the `authorization` field of a client's own requests. Its `Debug` hides it, and it has no `Display`, so that it is not printed by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// `text`, without the whitespace around it, or `None` when that is empty or holds anything but visible ASCII, which an `authorization` field cannot carry.
    ///
    /// ```
    /// use tomori_prometheus::Token;
    ///
    /// assert!(Token::new(" eyJhbGciOi.payload.signature\n").is_some());
    /// assert!(Token::new("two words").is_none());
    /// ```
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_graphic()))
            .then(|| Self(text.to_owned()))
    }

    /// The token's text, for the `authorization` field and for removing it from what errors quote.
    pub(crate) fn secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token(..)")
    }
}

/// Where a client gets its bearer token, which the application implements, such as by running a command or reading a file.
///
/// A client asks for a token when it has none, when the one it kept has outlived the client's token lifetime, and once after the endpoint refused it with HTTP 401. The source may block, as a command does; it is called on the caller's thread, while other callers of the same client wait for it. Its error is reported as an authentication failure, to whoever the server side shows its errors to, so it must not contain a token.
///
/// A fixed [`Token`] is a source of itself, and so is a closure:
///
/// ```
/// use tomori_prometheus::{Token, TokenSource};
///
/// let source = || {
///     let text = std::env::var("PROMETHEUS_TOKEN").map_err(|_| "set PROMETHEUS_TOKEN".to_owned())?;
///     Token::new(&text).ok_or_else(|| "PROMETHEUS_TOKEN is not a token".to_owned())
/// };
/// let _: &dyn TokenSource = &source;
/// ```
pub trait TokenSource: Send + Sync {
    /// A new token, or why there is none.
    fn token(&self) -> Result<Token, String>;
}

impl TokenSource for Token {
    fn token(&self) -> Result<Token, String> {
        Ok(self.clone())
    }
}

impl<F> TokenSource for F
where
    F: Fn() -> Result<Token, String> + Send + Sync,
{
    fn token(&self) -> Result<Token, String> {
        self()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_visible_ascii_and_never_printed() {
        let token = Token::new("  secret-1 ").unwrap();
        assert_eq!(token.secret(), "secret-1");
        assert_eq!(format!("{token:?}"), "Token(..)");
        assert_eq!(Token::new(""), None);
        assert_eq!(Token::new("   "), None);
        assert_eq!(Token::new("caf\u{e9}"), None);
        assert_eq!(Token::new("a\u{7}b"), None);
        assert_eq!(token.token(), Ok(token.clone()));
        let source = || Err::<Token, _>("no token".to_owned());
        assert_eq!(TokenSource::token(&source), Err("no token".to_owned()));
    }
}
