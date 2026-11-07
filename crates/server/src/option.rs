// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A command-line option of an application's own, which [`crate::Builder::option`] adds to the `tomori-server` command line, such as an environment's configuration. See `design/builders.md`.
use std::rc::Rc;

use crate::Server;

/// Reads an option's value, or says why it cannot.
type Check = Box<dyn Fn(&str) -> Result<(), String>>;

/// Changes the configured server as an option's value says.
type Apply = Box<dyn Fn(Server, &str) -> Result<Server, String>>;

/// A command-line option of an application's own, `FLAG VALUE`, such as the GitHub environment's `--github-repository OWNER/NAME`: listed in `--help` after Tomori's own options, read as the command line is, and applied to the server once the command line has configured it, before the builder's hooks. Given more than once, the last is used.
///
/// ```
/// use tomori_server::{CommandOption, Environment, Server};
///
/// let fleet = CommandOption::new(
///     "--fleet",
///     "NAME",
///     "Offer the Observe environment, showing this fleet.",
///     |value| match value {
///         "" => Err("--fleet needs a name".to_owned()),
///         name => Ok(name.to_owned()),
///     },
///     |server, _fleet| Ok(server.with_environments([Environment::new("observe", "Observe")])),
/// );
/// let builder = Server::builder().option(fleet);
/// ```
pub struct CommandOption {
    flag: String,
    value: String,
    help: String,
    check: Check,
    apply: Apply,
}

impl CommandOption {
    /// The option `flag`, such as `--github-repository`, taking a value described as `value`, such as `OWNER/NAME`, and described in `--help` by `help`, whose lines are shown as they are, indented as Tomori's own options are. `parse` reads its value as the command line is read, failing with a message that the server reports as it stops, and `apply` changes the configured server as the value says, failing likewise.
    pub fn new<T: 'static>(
        flag: impl Into<String>,
        value: impl Into<String>,
        help: impl Into<String>,
        parse: impl Fn(&str) -> Result<T, String> + 'static,
        apply: impl Fn(Server, T) -> Result<Server, String> + 'static,
    ) -> Self {
        let parse = Rc::new(parse);
        let check = {
            let parse = parse.clone();
            Box::new(move |value: &str| parse(value).map(drop))
        };
        Self {
            flag: flag.into(),
            value: value.into(),
            help: help.into(),
            check,
            apply: Box::new(move |server, value| {
                parse(value).and_then(|value| apply(server, value))
            }),
        }
    }

    /// The flag, such as `--github-repository`.
    pub fn flag(&self) -> &str {
        &self.flag
    }

    /// What its value is, as `--help` names it, such as `OWNER/NAME`.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// What `--help` says of it.
    pub fn help(&self) -> &str {
        &self.help
    }

    /// Whether `value` can be read.
    pub(crate) fn check(&self, value: &str) -> Result<(), String> {
        (self.check)(value)
    }

    /// `server`, changed as `value` says.
    pub(crate) fn apply(&self, server: Server, value: &str) -> Result<Server, String> {
        (self.apply)(server, value)
    }
}
