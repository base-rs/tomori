// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Actions a server side declares on what it describes, which the space's agent takes when someone asks, such as
//! showing a terminal's charts for another region: see [`Action`], [`crate::Space::declare_actions`] and
//! [`crate::Object::offering`]. See `design/environment-actions.md`.
use std::collections::HashSet;

use crate::{Error, Result, valid_reference};

/// The most actions a side declares with [`crate::Space::declare_actions`]: a declaration of more is refused.
pub const ACTION_LIMIT: usize = 32;

/// The most actions one object offers with [`crate::Object::offering`]: those after are left out.
pub const OFFER_LIMIT: usize = 8;

/// The most parameters of one action: an action with more is refused.
pub const PARAMETER_LIMIT: usize = 4;

/// The most choices of one [`Parameter::choice`]: a parameter with more is refused. Enough for a list of regions.
pub const CHOICE_LIMIT: usize = 64;

/// The longest name of an action or a parameter, in bytes.
pub const ACTION_NAME_BYTES: usize = 32;

/// The longest description of an action told to the agent, in characters, once the server has cleaned it onto one
/// line: a longer one is cut to this many, followed by the `…` that marks it cut.
pub const ACTION_DESCRIPTION_LIMIT: usize = 200;

/// The longest description of a parameter told to the agent, in characters, cleaned and cut as an action's is.
pub const PARAMETER_DESCRIPTION_LIMIT: usize = 120;

/// The longest label of a choice told to the agent, in characters, cleaned and cut as an action's description is.
pub const CHOICE_LABEL_LIMIT: usize = 80;

/// The longest text a [`Parameter::text`] takes, in characters.
pub const TEXT_LIMIT: usize = 200;

/// The most text in a declaration, in bytes: every name, description, value and label of every action together. A
/// declaration of more is refused, so that what the agent is told of an object's actions stays small.
pub const ACTIONS_LIMIT: usize = 32 * 1024;

/// The longest summary or reason of an [`Outcome`] told to the agent, in characters, once the server has cleaned it
/// onto one line: a longer one is cut to this many, followed by the `…` that marks it cut.
pub const OUTCOME_LIMIT: usize = 200;

/// Whether `name` is one a side may name an action or a parameter by: 1 to [`ACTION_NAME_BYTES`] bytes of lowercase
/// ASCII letters, digits and `_`, beginning with a letter, as the agent reads a name.
///
/// ```
/// use tomori_environment::valid_action_name;
///
/// assert!(valid_action_name("set_region"));
/// assert!(!valid_action_name(""));
/// assert!(!valid_action_name("Set_region"));
/// assert!(!valid_action_name("set-region"));
/// assert!(!valid_action_name("2fast"));
/// ```
pub fn valid_action_name(name: &str) -> bool {
    (1..=ACTION_NAME_BYTES).contains(&name.len())
        && name.starts_with(|first: char| first.is_ascii_lowercase())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Something the space's agent may do to what the side describes, as the side declares it: its name, a description
/// written for the agent, its parameters, and whether it is consequential. Declared once with
/// [`crate::Space::declare_actions`], and offered by each object it applies to, by name, with
/// [`crate::Object::offering`].
///
/// An action is harmless unless the side marks it [consequential](Action::consequential): it changes only what the
/// space shows, for everyone in it, and can be changed back, as a terminal's region can, so the agent takes it in a
/// turn a participant's words began without naming who asked. A consequential one, which reaches outside the space,
/// cannot be undone or costs something, needs the agent to name who asked, an owner or a collaborator, whom the server
/// checks. See `design/environment-actions.md`.
///
/// ```
/// use tomori_environment::{Action, Choice, Parameter};
///
/// let action = Action::new("set_region", "Show this terminal's charts for another region")
///     .with(Parameter::choice("region", "the region to show", [Choice::new("eu-belgium").labelled("Belgium")]));
/// assert_eq!(action.name(), "set_region");
/// assert_eq!(action.parameters().len(), 1);
/// assert!(!action.is_consequential());
/// assert!(Action::new("restart", "Restart this pod").consequential().is_consequential());
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Action {
    name: String,
    description: String,
    parameters: Vec<Parameter>,
    consequential: bool,
}

impl Action {
    /// The harmless action `name`, described to the agent as `description`, without parameters.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: Vec::new(),
            consequential: false,
        }
    }

    /// The action, taking `parameter` after those it takes already. Every parameter is required.
    pub fn with(mut self, parameter: Parameter) -> Self {
        self.parameters.push(parameter);
        self
    }

    /// The action, marked consequential: it reaches outside the space, cannot be undone or costs something, so the
    /// server lets the agent take it only for an owner or a collaborator who asked, whom it names and the server checks.
    pub fn consequential(mut self) -> Self {
        self.consequential = true;
        self
    }

    /// Its name, as the agent calls it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the agent is told it does.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Its parameters, in order.
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    /// Whether it is consequential.
    pub fn is_consequential(&self) -> bool {
        self.consequential
    }

    /// The action with its description, its parameters' and its choices' labels each made by `clean` from the text and
    /// the most characters it may keep, [`ACTION_DESCRIPTION_LIMIT`], [`PARAMETER_DESCRIPTION_LIMIT`] or
    /// [`CHOICE_LABEL_LIMIT`], as the server cleans them onto one line and cuts them before telling the agent.
    ///
    /// ```
    /// use tomori_environment::{Action, Choice, Parameter};
    ///
    /// let action = Action::new("go", "Go\nsomewhere").with(Parameter::choice("to", "where", [Choice::new("a").labelled("A")]));
    /// let cleaned = action.cleaned(|text, _| text.replace('\n', " "));
    /// assert_eq!(cleaned.description(), "Go somewhere");
    /// ```
    pub fn cleaned(&self, clean: impl Fn(&str, usize) -> String) -> Self {
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| Parameter {
                name: parameter.name.clone(),
                description: clean(&parameter.description, PARAMETER_DESCRIPTION_LIMIT),
                kind: match &parameter.kind {
                    Kind::Choice(choices) => Kind::Choice(
                        choices
                            .iter()
                            .map(|choice| Choice {
                                value: choice.value.clone(),
                                label: choice
                                    .label
                                    .as_deref()
                                    .map(|label| clean(label, CHOICE_LABEL_LIMIT)),
                            })
                            .collect(),
                    ),
                    other => other.clone(),
                },
            })
            .collect();
        Self {
            name: self.name.clone(),
            description: clean(&self.description, ACTION_DESCRIPTION_LIMIT),
            parameters,
            consequential: self.consequential,
        }
    }
}

/// A parameter of an [`Action`]: its name, a description written for the agent, and the values it takes.
///
/// ```
/// use tomori_environment::{Kind, Parameter};
///
/// let pecks = Parameter::number("pecks", "how many times she pecks", 1.0, 8.0).stepped(1.0);
/// assert_eq!(pecks.name(), "pecks");
/// assert_eq!(pecks.kind(), &Kind::Number { least: 1.0, most: 8.0, step: Some(1.0) });
/// let note = Parameter::text("note", "a few words", 40);
/// assert_eq!(note.kind(), &Kind::Text { chars: 40 });
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Parameter {
    name: String,
    description: String,
    kind: Kind,
}

/// The values a [`Parameter`] takes.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// One of these choices, by its value.
    Choice(Vec<Choice>),
    /// A number from `least` to `most`, both included, on `step` from `least`, if it has one.
    Number {
        least: f64,
        most: f64,
        step: Option<f64>,
    },
    /// Text of at most `chars` characters, cleaned onto one line.
    Text { chars: usize },
}

impl Parameter {
    /// The parameter `name`, described as `description`, taking one of `choices`, at most [`CHOICE_LIMIT`].
    pub fn choice(
        name: impl Into<String>,
        description: impl Into<String>,
        choices: impl IntoIterator<Item = Choice>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            kind: Kind::Choice(choices.into_iter().collect()),
        }
    }

    /// The parameter `name`, described as `description`, taking a number from `least` to `most`, both included.
    pub fn number(
        name: impl Into<String>,
        description: impl Into<String>,
        least: f64,
        most: f64,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            kind: Kind::Number {
                least,
                most,
                step: None,
            },
        }
    }

    /// The parameter, a number taking only `least` and each `step` above it, such as whole numbers with a step of 1.
    /// A parameter that is not a number is unchanged.
    pub fn stepped(mut self, step: f64) -> Self {
        if let Kind::Number { step: stepped, .. } = &mut self.kind {
            *stepped = Some(step);
        }
        self
    }

    /// The parameter `name`, described as `description`, taking text of at most `chars` characters, at most
    /// [`TEXT_LIMIT`], which the server cleans onto one line.
    pub fn text(name: impl Into<String>, description: impl Into<String>, chars: usize) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            kind: Kind::Text { chars },
        }
    }

    /// Its name, as the agent gives it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the agent is told it is.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The values it takes.
    pub fn kind(&self) -> &Kind {
        &self.kind
    }
}

/// One of the values a [`Parameter::choice`] takes: the value the agent gives and the side is handed, such as
/// `eu-belgium`, and optionally a label for matching what people say, such as `Belgium (eu-west-3)`.
///
/// ```
/// use tomori_environment::Choice;
///
/// let choice = Choice::new("eu-belgium").labelled("Belgium");
/// assert_eq!((choice.value(), choice.label()), ("eu-belgium", Some("Belgium")));
/// assert_eq!(Choice::new("us-east-1").label(), None);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Choice {
    value: String,
    label: Option<String>,
}

impl Choice {
    /// The choice `value`, which [`valid_reference`] must take.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: None,
        }
    }

    /// The choice, labelled `label` for the agent.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Its value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Its label, if it has one.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// Whether `actions` is a declaration the server takes, as [`crate::Space::declare_actions`] is refused otherwise,
/// saying why: at most [`ACTION_LIMIT`] actions, each named as [`valid_action_name`] takes and unlike the others, with
/// at most [`PARAMETER_LIMIT`] parameters, each named so and unlike the action's others; a choice of 1 to
/// [`CHOICE_LIMIT`] values, each one [`valid_reference`] takes and unlike the others; a number whose least and most
/// are finite, the least no more than the most, and whose step, if it has one, is finite and above 0; text of 1 to
/// [`TEXT_LIMIT`] characters; and at most [`ACTIONS_LIMIT`] bytes of text in all. A side's tests may check its own.
///
/// ```
/// use tomori_environment::{Action, Parameter, check_actions};
///
/// assert!(check_actions(&[Action::new("peck", "Peck at the ground")]).is_ok());
/// let twice = [Action::new("peck", "a"), Action::new("peck", "b")];
/// assert_eq!(check_actions(&twice).unwrap_err().message(), "Two actions are named peck.");
/// ```
pub fn check_actions(actions: &[Action]) -> Result<()> {
    if actions.len() > ACTION_LIMIT {
        return Err(Error::new(format!(
            "{} actions are declared, more than the {ACTION_LIMIT} the server takes.",
            actions.len()
        )));
    }
    let mut names = HashSet::new();
    let mut bytes = 0;
    for action in actions {
        let name = action.name();
        if !valid_action_name(name) {
            return Err(Error::new(
                "An action is named other than with 1 to 32 lowercase letters, digits and _, beginning with a letter.",
            ));
        }
        if !names.insert(name) {
            return Err(Error::new(format!("Two actions are named {name}.")));
        }
        if action.parameters().len() > PARAMETER_LIMIT {
            return Err(Error::new(format!(
                "The action {name} has {} parameters, more than the {PARAMETER_LIMIT} the server takes.",
                action.parameters().len()
            )));
        }
        bytes += name.len() + action.description().len();
        let mut parameters = HashSet::new();
        for parameter in action.parameters() {
            let named = parameter.name();
            if !valid_action_name(named) {
                return Err(Error::new(format!(
                    "A parameter of the action {name} is named other than with 1 to 32 lowercase letters, digits and _, beginning with a letter."
                )));
            }
            if !parameters.insert(named) {
                return Err(Error::new(format!(
                    "Two parameters of the action {name} are named {named}."
                )));
            }
            bytes += named.len() + parameter.description().len();
            match parameter.kind() {
                Kind::Choice(choices) => {
                    if choices.is_empty() || choices.len() > CHOICE_LIMIT {
                        return Err(Error::new(format!(
                            "The parameter {named} of the action {name} has {} choices, not from 1 to {CHOICE_LIMIT}.",
                            choices.len()
                        )));
                    }
                    let mut values = HashSet::new();
                    for choice in choices {
                        if !valid_reference(choice.value()) {
                            return Err(Error::new(format!(
                                "A choice of the parameter {named} of the action {name} has a value other than 1 to 64 letters, digits, -, _, . and /."
                            )));
                        }
                        if !values.insert(choice.value()) {
                            return Err(Error::new(format!(
                                "Two choices of the parameter {named} of the action {name} are {}.",
                                choice.value()
                            )));
                        }
                        bytes += choice.value().len() + choice.label().map_or(0, str::len);
                    }
                }
                Kind::Number { least, most, step } => {
                    let stepped = step.is_none_or(|step| step.is_finite() && step > 0.0);
                    if !least.is_finite() || !most.is_finite() || least > most || !stepped {
                        return Err(Error::new(format!(
                            "The parameter {named} of the action {name} is a number whose bounds or step the server does not take."
                        )));
                    }
                }
                Kind::Text { chars } => {
                    if !(1..=TEXT_LIMIT).contains(chars) {
                        return Err(Error::new(format!(
                            "The parameter {named} of the action {name} takes text of {chars} characters, not from 1 to {TEXT_LIMIT}."
                        )));
                    }
                }
            }
        }
    }
    if bytes > ACTIONS_LIMIT {
        return Err(Error::new(format!(
            "The actions declared hold {bytes} bytes of text, more than the {ACTIONS_LIMIT} the server takes."
        )));
    }
    Ok(())
}

/// An action the space's agent took, as the server hands it to the side, in an [`crate::Intent`] whose
/// [`crate::Intent::acted`] it is: the reference of the object it is taken on, the action's name, its parameters, as
/// a JSON object the server checked against the declaration, every parameter present, nothing else, and each a value
/// it takes, and the ticket the side answers it by, with [`crate::Space::answer`].
///
/// ```
/// use tomori_environment::Acted;
///
/// let acted = Acted::new(7, "terminal/2", "set_region", r#"{"region":"eu-belgium"}"#);
/// assert_eq!((acted.ticket(), acted.reference(), acted.action()), (7, "terminal/2", "set_region"));
/// assert_eq!(acted.parameters(), r#"{"region":"eu-belgium"}"#);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Acted {
    ticket: u64,
    reference: String,
    action: String,
    parameters: String,
}

impl Acted {
    /// The action `action`, taken on the object of `reference` with `parameters`, a JSON object, answered by `ticket`.
    /// The server makes it; a side's own tests make it to try their handling of an action.
    pub fn new(
        ticket: u64,
        reference: impl Into<String>,
        action: impl Into<String>,
        parameters: impl Into<String>,
    ) -> Self {
        Self {
            ticket,
            reference: reference.into(),
            action: action.into(),
            parameters: parameters.into(),
        }
    }

    /// The number the side answers it by.
    pub fn ticket(&self) -> u64 {
        self.ticket
    }

    /// The reference of the object it is taken on, as the side described it.
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// The action's name, as the side declared it.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Its parameters, as a JSON object of each parameter's name and value: a choice's value as a string, a number as
    /// a number and text as a string, cleaned onto one line.
    pub fn parameters(&self) -> &str {
        &self.parameters
    }
}

/// How a side answers an action, with [`crate::Space::answer`]: done, with a summary of what changed, such as
/// "Terminal 2 now shows Belgium", or refused, with the reason, such as "Terminal 2 is frozen while its incident is
/// open". The agent is told it, and the space's status wall shows it, cleaned onto one line and cut to
/// [`OUTCOME_LIMIT`] characters, as data, never as instructions.
///
/// ```
/// use tomori_environment::Outcome;
///
/// let done = Outcome::done("Terminal 2 now shows Belgium");
/// assert!(done.is_done());
/// assert_eq!(done.message(), "Terminal 2 now shows Belgium");
/// assert!(!Outcome::refused("Terminal 2 is frozen").is_done());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Outcome {
    done: bool,
    message: String,
}

impl Outcome {
    /// The action was done, as `summary` says.
    pub fn done(summary: impl Into<String>) -> Self {
        Self {
            done: true,
            message: summary.into(),
        }
    }

    /// The action was refused, for `reason`.
    pub fn refused(reason: impl Into<String>) -> Self {
        Self {
            done: false,
            message: reason.into(),
        }
    }

    /// Whether it was done.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The summary of what was done, or why it was refused.
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(choices: usize) -> Action {
        Action::new("set_region", "Show another region").with(Parameter::choice(
            "region",
            "the region",
            (0..choices).map(|index| Choice::new(format!("r{index}")).labelled("Region")),
        ))
    }

    fn refused(actions: &[Action]) -> String {
        check_actions(actions).unwrap_err().message().to_owned()
    }

    #[test]
    fn a_declaration_within_its_limits_is_taken() {
        let actions: Vec<Action> = (0..ACTION_LIMIT)
            .map(|index| {
                let mut action = Action::new(format!("a{index}"), "something");
                for parameter in 0..PARAMETER_LIMIT {
                    action =
                        action.with(Parameter::text(format!("p{parameter}"), "text", TEXT_LIMIT));
                }
                action
            })
            .collect();
        assert_eq!(check_actions(&actions), Ok(()));
        assert_eq!(check_actions(&[region(CHOICE_LIMIT)]), Ok(()));
        let pecks = Parameter::number("pecks", "how many", 1.0, 8.0).stepped(1.0);
        assert_eq!(
            check_actions(&[Action::new("peck", "Peck").with(pecks).consequential()]),
            Ok(())
        );
        // A step is only a number's.
        assert_eq!(
            Parameter::text("note", "text", 4).stepped(1.0).kind(),
            &Kind::Text { chars: 4 }
        );
    }

    #[test]
    fn a_declaration_past_a_limit_or_badly_named_is_refused_saying_why() {
        let many: Vec<Action> = (0..=ACTION_LIMIT)
            .map(|index| Action::new(format!("a{index}"), ""))
            .collect();
        assert_eq!(
            refused(&many),
            "33 actions are declared, more than the 32 the server takes."
        );
        assert!(refused(&[Action::new("Peck", "")]).starts_with("An action is named"));
        assert_eq!(
            refused(&[Action::new("peck", "a"), Action::new("peck", "b")]),
            "Two actions are named peck."
        );
        assert!(refused(&[Action::new("a".repeat(33), "")]).starts_with("An action is named"));
        let mut crowded = Action::new("crowded", "");
        for parameter in 0..=PARAMETER_LIMIT {
            crowded = crowded.with(Parameter::text(format!("p{parameter}"), "", 1));
        }
        assert_eq!(
            refused(&[crowded]),
            "The action crowded has 5 parameters, more than the 4 the server takes."
        );
        let badly = Action::new("act", "").with(Parameter::text("Bad", "", 1));
        assert!(refused(&[badly]).starts_with("A parameter of the action act is named"));
        let twice = Action::new("act", "")
            .with(Parameter::text("p", "", 1))
            .with(Parameter::text("p", "", 1));
        assert_eq!(
            refused(&[twice]),
            "Two parameters of the action act are named p."
        );
        assert_eq!(
            refused(&[region(0)]),
            "The parameter region of the action set_region has 0 choices, not from 1 to 64."
        );
        assert_eq!(
            refused(&[region(CHOICE_LIMIT + 1)]),
            "The parameter region of the action set_region has 65 choices, not from 1 to 64."
        );
        let spaced = Action::new("go", "").with(Parameter::choice("to", "", [Choice::new("a b")]));
        assert!(refused(&[spaced]).contains("has a value other than"));
        let same = Action::new("go", "").with(Parameter::choice(
            "to",
            "",
            [Choice::new("a"), Choice::new("a")],
        ));
        assert_eq!(
            refused(&[same]),
            "Two choices of the parameter to of the action go are a."
        );
        for number in [
            Parameter::number("n", "", f64::NAN, 1.0),
            Parameter::number("n", "", 0.0, f64::INFINITY),
            Parameter::number("n", "", 2.0, 1.0),
            Parameter::number("n", "", 0.0, 1.0).stepped(0.0),
            Parameter::number("n", "", 0.0, 1.0).stepped(f64::NAN),
        ] {
            assert!(
                refused(&[Action::new("set", "").with(number)])
                    .contains("whose bounds or step the server does not take")
            );
        }
        for chars in [0, TEXT_LIMIT + 1] {
            assert!(
                refused(&[Action::new("say", "").with(Parameter::text("t", "", chars))])
                    .contains("not from 1 to 200")
            );
        }
        // Text in all, here 32 actions of 1,100 bytes of description each.
        let wordy: Vec<Action> = (0..ACTION_LIMIT)
            .map(|index| Action::new(format!("a{index}"), "x".repeat(1_100)))
            .collect();
        assert!(refused(&wordy).ends_with("more than the 32768 the server takes."));
    }

    #[test]
    fn an_action_taken_and_its_outcome_are_what_they_were_given() {
        let acted = Acted::new(3, "hen/pip", "peck", r#"{"pecks":2}"#);
        assert_eq!(
            (
                acted.ticket(),
                acted.reference(),
                acted.action(),
                acted.parameters()
            ),
            (3, "hen/pip", "peck", r#"{"pecks":2}"#)
        );
        let refused = Outcome::refused("She is roosting.");
        assert_eq!(
            (refused.is_done(), refused.message()),
            (false, "She is roosting.")
        );
        let action = region(1);
        assert_eq!(action.description(), "Show another region");
        // Cleaned, each text with the most it may keep, and the rest as it was.
        let cut = |text: &str, most: usize| format!("{most}:{text}");
        let pecks = Parameter::number("pecks", "how many", 1.0, 8.0).stepped(1.0);
        let cleaned = region(1).with(pecks.clone()).consequential().cleaned(cut);
        assert_eq!(cleaned.description(), "200:Show another region");
        assert_eq!(cleaned.parameters()[0].description(), "120:the region");
        assert_eq!(
            cleaned.parameters()[0].kind(),
            &Kind::Choice(vec![Choice::new("r0").labelled("80:Region")])
        );
        assert_eq!(cleaned.parameters()[1].kind(), pecks.kind());
        assert!(cleaned.is_consequential());
        let plain =
            Action::new("go", "Go").with(Parameter::choice("to", "where", [Choice::new("a")]));
        assert_eq!(
            plain.cleaned(cut).parameters()[0].kind(),
            &Kind::Choice(vec![Choice::new("a")])
        );
        assert_eq!(action.parameters()[0].description(), "the region");
    }

    /// The limits are those `design/environment-actions.md` states, so that changing one is deliberate.
    #[test]
    fn the_limits_are_those_the_design_states() {
        assert_eq!(
            (ACTION_LIMIT, OFFER_LIMIT, PARAMETER_LIMIT, CHOICE_LIMIT),
            (32, 8, 4, 64)
        );
        assert_eq!((ACTION_NAME_BYTES, ACTION_DESCRIPTION_LIMIT), (32, 200));
        assert_eq!((PARAMETER_DESCRIPTION_LIMIT, CHOICE_LABEL_LIMIT), (120, 80));
        assert_eq!(
            (TEXT_LIMIT, ACTIONS_LIMIT, OUTCOME_LIMIT),
            (200, 32 * 1024, 200)
        );
    }
}
