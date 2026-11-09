// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A command-line option of an application's own, which [`crate::Builder::option`] adds to the `tomori-server` command line, such as an environment's configuration. See `design/builders.md`.
use std::cell::Cell;

use crate::Server;

/// An option's value, read from the command line and kept, in its own type, until it is applied.
trait Value {
    /// Read `value` and keep what it says, in place of any kept before, or say why it cannot be read.
    fn read(&self, value: &str) -> Result<(), String>;

    /// Drop the value kept, if any.
    fn forget(&self);

    /// `server`, changed as the value kept says, which is then dropped; unchanged if none is kept.
    fn apply(&self, server: Server) -> Result<Server, String>;
}

/// An option's `parse` and `apply`, with the value of type `T` that `parse` made, kept for `apply`.
struct Typed<T, P, A> {
    parse: P,
    apply: A,
    kept: Cell<Option<T>>,
}

impl<T, P, A> Value for Typed<T, P, A>
where
    P: Fn(&str) -> Result<T, String>,
    A: Fn(Server, T) -> Result<Server, String>,
{
    fn read(&self, value: &str) -> Result<(), String> {
        self.kept.set(Some((self.parse)(value)?));
        Ok(())
    }

    fn forget(&self) {
        self.kept.set(None);
    }

    fn apply(&self, server: Server) -> Result<Server, String> {
        match self.kept.take() {
            Some(value) => (self.apply)(server, value),
            None => Ok(server),
        }
    }
}

/// A command-line option of an application's own, `FLAG VALUE`, such as the GitHub environment's `--github-repository OWNER/NAME`: listed in `--help` after Tomori's own options, read as the command line is, and applied to the server once the command line has configured it, before the builder's hooks. Given more than once, the last is used. Its `parse` runs once for each time it is given, as the command line is read, and `apply` is given the value that `parse` made for the last, rather than one made again.
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
    typed: Box<dyn Value>,
}

impl CommandOption {
    /// The option `flag`, such as `--github-repository`, taking a value described as `value`, such as `OWNER/NAME`, and described in `--help` by `help`, whose lines are shown as they are, indented as Tomori's own options are. `parse` reads its value as the command line is read, once each time the flag is given, failing with a message that the server reports as it stops, and `apply` changes the configured server as the value `parse` made for the last says, failing likewise. `parse` may run for a command line that is then refused, such as for another argument, in which case `apply` does not run and its value is kept only until the next command line is read or the builder is dropped, which a refused command line ends.
    pub fn new<T: 'static>(
        flag: impl Into<String>,
        value: impl Into<String>,
        help: impl Into<String>,
        parse: impl Fn(&str) -> Result<T, String> + 'static,
        apply: impl Fn(Server, T) -> Result<Server, String> + 'static,
    ) -> Self {
        Self {
            flag: flag.into(),
            value: value.into(),
            help: help.into(),
            typed: Box::new(Typed {
                parse,
                apply,
                kept: Cell::new(None),
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

    /// Read `value`, keeping what it says for [`Self::apply`] in place of any value kept before, or say why it cannot be read.
    pub(crate) fn read(&self, value: &str) -> Result<(), String> {
        self.typed.read(value)
    }

    /// Drop the value kept, as a command line is about to be read.
    pub(crate) fn forget(&self) {
        self.typed.forget()
    }

    /// `server`, changed as the value kept says, which is then dropped, or unchanged if the option was not given.
    pub(crate) fn apply(&self, server: Server) -> Result<Server, String> {
        self.typed.apply(server)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use super::*;
    use crate::{Backends, Builder, Environment, NullTranscriber, command};

    /// A fleet `--fleet` names, with which of its parses made it.
    struct Fleet {
        name: String,
        made: u32,
    }

    /// How many times `--fleet` was parsed, and the fleets it was applied with, by name and which parse made each.
    #[derive(Default)]
    struct Counts {
        parsed: Rc<Cell<u32>>,
        applied: Rc<RefCell<Vec<(String, u32)>>>,
    }

    impl Counts {
        /// `--fleet NAME`, counted, refusing an empty name, and offering an environment labelled with the fleet's name.
        fn option(&self) -> CommandOption {
            let parsed = self.parsed.clone();
            let applied = self.applied.clone();
            CommandOption::new(
                "--fleet",
                "NAME",
                "Show this fleet.",
                move |value| {
                    parsed.set(parsed.get() + 1);
                    match value {
                        "" => Err("--fleet needs a name".to_owned()),
                        name => Ok(Fleet {
                            name: name.to_owned(),
                            made: parsed.get(),
                        }),
                    }
                },
                move |server, fleet| {
                    applied.borrow_mut().push((fleet.name.clone(), fleet.made));
                    Ok(server.with_environments([Environment::new("observe", &fleet.name)]))
                },
            )
        }
    }

    /// Read the command line `list` with `builder`'s options.
    fn read(builder: &Builder, list: &[&str]) -> Result<(), String> {
        command::parse(
            list.iter().map(|argument| argument.to_string()),
            &builder.usage(),
            builder.options(),
        )
        .map(drop)
    }

    /// The labels of the environments a server offers once `builder`'s options are applied to it.
    fn applied(builder: &Builder) -> Vec<String> {
        let server = Server::new(Backends::new(), Arc::new(NullTranscriber::new()));
        builder
            .options()
            .iter()
            .try_fold(server, |server, option| option.apply(server))
            .unwrap()
            .environment_labels()
    }

    #[test]
    fn an_options_value_is_parsed_once_and_given_to_apply() {
        let counts = Counts::default();
        let builder = Server::builder().option(counts.option());
        read(&builder, &["--fleet", "alpha"]).unwrap();
        assert_eq!(counts.parsed.get(), 1);
        // Applied with the value its one parse made, not one made again, which is then dropped.
        assert_eq!(applied(&builder), ["alpha"]);
        assert_eq!(counts.parsed.get(), 1);
        assert_eq!(*counts.applied.borrow(), [("alpha".to_owned(), 1)]);
        assert!(applied(&builder).is_empty());
        assert_eq!(counts.applied.borrow().len(), 1);
    }

    #[test]
    fn a_repeated_option_is_parsed_each_time_and_applied_with_the_last() {
        let counts = Counts::default();
        let builder = Server::builder().option(counts.option());
        read(&builder, &["--fleet", "alpha", "--fleet", "beta"]).unwrap();
        assert_eq!(counts.parsed.get(), 2);
        assert_eq!(applied(&builder), ["beta"]);
        assert_eq!(counts.parsed.get(), 2);
        assert_eq!(*counts.applied.borrow(), [("beta".to_owned(), 2)]);
    }

    #[test]
    fn an_option_that_cannot_be_parsed_refuses_the_command_line_and_is_not_applied() {
        let counts = Counts::default();
        let builder = Server::builder().option(counts.option());
        assert_eq!(
            read(&builder, &["--fleet", ""]),
            Err("--fleet needs a name".to_owned())
        );
        assert_eq!(counts.parsed.get(), 1);
        assert!(applied(&builder).is_empty());
        assert!(counts.applied.borrow().is_empty());
    }

    #[test]
    fn an_option_keeps_only_the_value_of_the_command_line_read_last() {
        let counts = Counts::default();
        let builder = Server::builder().option(counts.option());
        // Not given, it is neither parsed nor applied.
        read(&builder, &[]).unwrap();
        assert!(applied(&builder).is_empty());
        // Given on a command line read before another without it, its value is dropped.
        read(&builder, &["--fleet", "alpha"]).unwrap();
        read(&builder, &[]).unwrap();
        assert!(applied(&builder).is_empty());
        assert_eq!(counts.parsed.get(), 1);
        assert!(counts.applied.borrow().is_empty());
    }
}
