// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the agent asks for a panel to go: a `place` header line in a panel block, or a `place=` attribute in a code
//! block's info string, in one of a few forms that name a person, a wall or neither. Everything in them is untrusted,
//! so a form the server does not know is ignored, and the panel goes where it would without one. See
//! `design/projection.md`.
//!
//! ````text
//! ```panel
//! title: Build status
//! place: near Ada
//! ---
//! All green.
//! ```
//!
//! ```rust place=looking:Ada
//! fn main() {}
//! ```
//! ````

/// The longest placement taken, in characters, after which it is cut: more than a display name and a wall's name.
const PLACE_CHARS: usize = 80;

/// Where the agent asks for a panel to go. A form without a name is about whoever asked, as they began speaking.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// `near [NAME]`, or `nearest`: onto the wall nearest them.
    Near(Option<String>),
    /// `looking [NAME]`: onto the wall they are looking at, in the slot they look at if it is free, or else the wall in
    /// their view nearest where they look, or else the wall nearest them.
    Looking(Option<String>),
    /// `front [NAME]`: floating in front of them, facing them.
    Front(Option<String>),
    /// `wall WALL`: onto the wall with that identity, or that name, such as `Tomori wall` or `added wall 1`; and, with
    /// `slot N` after it, into that slot, numbered from 0 as `presence` numbers them, if it is free.
    Wall(String, Option<u32>),
}

impl Placement {
    /// The placement `value` names, such as `near Ada`, `near:Ada`, `nearest`, `looking`, `front Bob` or `wall tomori`,
    /// its form in any case and separated from what it names by spaces or a colon; or `None` for one it does not know,
    /// or `wall` naming none.
    ///
    /// ```text
    /// near Ada        -> Near(Some("Ada"))
    /// nearest         -> Near(None)
    /// looking:Bob     -> Looking(Some("Bob"))
    /// wall added wall 1 -> Wall("added wall 1", None)
    /// wall added wall 1 slot 2 -> Wall("added wall 1", Some(2))
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        let value = tomori_protocol::text::one_line(value, PLACE_CHARS);
        let value = value.trim();
        let (form, rest) = match value.find(|c: char| c == ':' || c.is_whitespace()) {
            Some(at) => (&value[..at], value[at..].trim_start_matches(':').trim()),
            None => (value, ""),
        };
        let named = (!rest.is_empty()).then(|| rest.to_owned());
        match form.to_ascii_lowercase().as_str() {
            "near" | "nearest" => Some(Self::Near(named)),
            "looking" | "look" => Some(Self::Looking(named)),
            "front" => Some(Self::Front(named)),
            "wall" => named.map(|named| {
                let (wall, slot) = slotted(&named);
                Self::Wall(wall.to_owned(), slot)
            }),
            _ => None,
        }
    }

    /// Split `info`, a code block's info string, into the rest of it and the placement its `place=` attribute names, if
    /// any: `place=FORM:NAME`, or `place="FORM NAME"` in double quotes, which may hold spaces. The attribute is taken
    /// out of the rest, so that it is not part of the panel's title or language, whether it is known or not.
    pub fn from_info(info: &str) -> (String, Option<Self>) {
        let Some(start) = attribute(info) else {
            return (info.to_owned(), None);
        };
        let value_at = start + "place=".len();
        let after = &info[value_at..];
        let (value, end) = match after.strip_prefix('"') {
            Some(quoted) => match quoted.find('"') {
                Some(close) => (&quoted[..close], value_at + close + 2),
                None => (quoted, info.len()),
            },
            None => {
                let length = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..length], value_at + length)
            }
        };
        let rest = format!("{} {}", info[..start].trim_end(), info[end..].trim_start());
        (rest.trim().to_owned(), Self::parse(value))
    }
}

/// `named`, a wall's identity or name, and the slot `slot N` after it says, if it says one: `slot` in any case, and
/// the number, separated by spaces or colons, as in `added wall 1 slot 2` or `wall-3:slot:2`.
fn slotted(named: &str) -> (&str, Option<u32>) {
    let separated = |c: char| c == ':' || c.is_whitespace();
    let Some((before, number)) = named.trim_end().rsplit_once(separated) else {
        return (named, None);
    };
    let Ok(slot) = number.parse::<u32>() else {
        return (named, None);
    };
    let before = before.trim_end_matches(separated);
    match before
        .len()
        .checked_sub(4)
        .filter(|at| before.is_char_boundary(*at))
    {
        Some(at)
            if before[at..].eq_ignore_ascii_case("slot")
                && before[..at].ends_with(separated)
                && !before[..at].trim_end_matches(separated).is_empty() =>
        {
            (before[..at].trim_end_matches(separated), Some(slot))
        }
        _ => (named, None),
    }
}

/// Where in `info` its `place=` attribute starts: a word of its own, in any case.
fn attribute(info: &str) -> Option<usize> {
    info.char_indices()
        .filter(|(at, c)| {
            !c.is_whitespace() && (*at == 0 || info[..*at].ends_with(char::is_whitespace))
        })
        .map(|(at, _)| at)
        .find(|at| {
            info[*at..]
                .get(..6)
                .is_some_and(|word| word.eq_ignore_ascii_case("place="))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Option<String> {
        Some(name.to_owned())
    }

    #[test]
    fn a_placement_names_its_form_and_whom_or_which_wall_it_is_about() {
        assert_eq!(
            Placement::parse("near Ada"),
            Some(Placement::Near(named("Ada")))
        );
        assert_eq!(
            Placement::parse("near:Ada"),
            Some(Placement::Near(named("Ada")))
        );
        assert_eq!(
            Placement::parse("  Near:  Ada Lovelace "),
            Some(Placement::Near(named("Ada Lovelace")))
        );
        assert_eq!(Placement::parse("nearest"), Some(Placement::Near(None)));
        assert_eq!(Placement::parse("near"), Some(Placement::Near(None)));
        assert_eq!(Placement::parse("LOOKING"), Some(Placement::Looking(None)));
        assert_eq!(
            Placement::parse("look Bob"),
            Some(Placement::Looking(named("Bob")))
        );
        assert_eq!(
            Placement::parse("front:Bob"),
            Some(Placement::Front(named("Bob")))
        );
        assert_eq!(Placement::parse("front"), Some(Placement::Front(None)));
        assert_eq!(
            Placement::parse("wall tomori"),
            Some(Placement::Wall("tomori".into(), None))
        );
        assert_eq!(
            Placement::parse("wall: added wall 1"),
            Some(Placement::Wall("added wall 1".into(), None))
        );
        // A wall and a slot of it, by name or identity, with spaces or colons.
        assert_eq!(
            Placement::parse("wall added wall 1 slot 2"),
            Some(Placement::Wall("added wall 1".into(), Some(2)))
        );
        assert_eq!(
            Placement::parse("wall:wall-3:SLOT:0"),
            Some(Placement::Wall("wall-3".into(), Some(0)))
        );
        assert_eq!(
            Placement::parse("Wall the Tomori wall  slot  5"),
            Some(Placement::Wall("the Tomori wall".into(), Some(5)))
        );
        // A number that is not after slot, or slot without a wall, is part of the wall's name.
        for named in [
            "added wall 1",
            "slot 2",
            "timeslot 2",
            "added wall slot x",
            "é slot 9999999999",
            "x",
        ] {
            assert_eq!(
                Placement::parse(&format!("wall {named}")),
                Some(Placement::Wall(named.into(), None)),
                "{named}"
            );
        }
        // A form it does not know, or a wall naming none, is ignored.
        assert_eq!(Placement::parse("wall"), None);
        assert_eq!(Placement::parse("behind Ada"), None);
        assert_eq!(Placement::parse(""), None);
        // Hostile text is cleaned onto one line and bounded.
        assert_eq!(
            Placement::parse("near Ada\nwall tomori\u{202e}"),
            Some(Placement::Near(named("Ada wall tomori")))
        );
        let long = format!("near {}", "a".repeat(200));
        assert!(matches!(
            Placement::parse(&long),
            Some(Placement::Near(Some(name))) if name.chars().count() < PLACE_CHARS
        ));
    }

    #[test]
    fn a_code_block_s_info_string_names_its_placement_as_an_attribute_taken_out_of_it() {
        assert_eq!(
            Placement::from_info("rust place=near:Ada"),
            ("rust".to_owned(), Some(Placement::Near(named("Ada"))))
        );
        assert_eq!(
            Placement::from_info("python place=\"looking Ada Lovelace\" title=area.py"),
            (
                "python title=area.py".to_owned(),
                Some(Placement::Looking(named("Ada Lovelace")))
            )
        );
        assert_eq!(
            Placement::from_info("Place=nearest"),
            (String::new(), Some(Placement::Near(None)))
        );
        // An attribute it does not know is still taken out; one left open runs to the end.
        assert_eq!(
            Placement::from_info("rust place=behind:Ada"),
            ("rust".to_owned(), None)
        );
        assert_eq!(
            Placement::from_info("rust place=\"wall added wall 1"),
            (
                "rust".to_owned(),
                Some(Placement::Wall("added wall 1".into(), None))
            )
        );
        assert_eq!(
            Placement::from_info("rust place=\"wall added wall 2 slot 3\""),
            (
                "rust".to_owned(),
                Some(Placement::Wall("added wall 2".into(), Some(3)))
            )
        );
        assert_eq!(
            Placement::from_info("rust place=wall:wall-4:slot:1 title=a.rs"),
            (
                "rust title=a.rs".to_owned(),
                Some(Placement::Wall("wall-4".into(), Some(1)))
            )
        );
        // Without one, or with one only inside another word, the info string is as it was.
        assert_eq!(Placement::from_info("rust"), ("rust".to_owned(), None));
        assert_eq!(
            Placement::from_info("rust replace=near"),
            ("rust replace=near".to_owned(), None)
        );
        assert_eq!(
            Placement::from_info("ünï place"),
            ("ünï place".to_owned(), None)
        );
    }
}
