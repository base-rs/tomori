// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fmt;

/// Why recognition or synthesis could not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The engine is not installed or configured; the message says how to set it up.
    Unavailable(String),
    /// The engine has stopped and serves no further requests until the server restarts: it panicked, its thread ended, or the server is shutting down.
    Stopped(String),
    /// The engine rejected the request.
    Engine(String),
    /// Recognition fell further behind the audio than [`crate::stt::BACKLOG`] allows.
    Behind,
    /// Recognition already holds as many utterances as it buffers at once, this many, so it takes no more until one of them has its outcome.
    Busy(usize),
}

impl Error {
    /// What sherpa-onnx loaded, or that it could not load `what`.
    pub(crate) fn loaded<T>(made: Option<T>, what: String) -> Result<T, Self> {
        made.ok_or(Self::Unavailable(format!(
            "sherpa-onnx could not load {what}"
        )))
    }

    /// What `engine` produced, or that it returned no result.
    pub(crate) fn produced<T>(result: Option<T>, engine: &str) -> Result<T, Self> {
        result.ok_or(Self::Engine(format!("{engine} returned no result")))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "speech engine unavailable: {message}"),
            Self::Stopped(message) => write!(formatter, "speech engine stopped: {message}"),
            Self::Engine(message) => write!(formatter, "speech engine error: {message}"),
            Self::Behind => write!(
                formatter,
                "speech recognition fell more than {} seconds behind the audio",
                crate::stt::BACKLOG / crate::stt::RATE as usize
            ),
            Self::Busy(count) => write!(
                formatter,
                "speech recognition is already taking {count} recordings, the most it holds at once"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_error_says_what_went_wrong() {
        for (error, message) in [
            (
                Error::Unavailable("install the model".into()),
                "speech engine unavailable: install the model".to_owned(),
            ),
            (
                Error::Stopped("it panicked".into()),
                "speech engine stopped: it panicked".to_owned(),
            ),
            (
                Error::Engine("bad input".into()),
                "speech engine error: bad input".to_owned(),
            ),
            (
                Error::Behind,
                format!(
                    "speech recognition fell more than {} seconds behind the audio",
                    crate::stt::BACKLOG / crate::stt::RATE as usize
                ),
            ),
            (
                Error::Busy(16),
                "speech recognition is already taking 16 recordings, the most it holds at once"
                    .to_owned(),
            ),
        ] {
            assert_eq!(error.to_string(), message);
        }
    }
}
