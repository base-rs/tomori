// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The list items of a text panel's body, as GitHub-flavoured Markdown writes them, a line at a time: bulleted with
//! `-`, `*` or `+`, or numbered with `1.` or `1)`, and task items, `- [ ]` and `- [x]`, among them, which the agent
//! ticks and unticks as a conversation covers them. Items are numbered from 1 in the order they are written, nested ones
//! included, as the agent and attention name them. Lines inside a fenced code block are not items, and neither is a
//! rule such as `- - -`. The server and clients read items by the same rule, so that the line a client points at, or
//! ticks, is the item the server ticks. See `design/projection.md`.

/// One list item of a body.
#[derive(Clone, Debug, PartialEq)]
pub struct Item<'a> {
    /// The line of the body it is on, from 0.
    pub line: usize,
    /// How deep it is nested, from 1.
    pub depth: usize,
    /// Whether it is ticked, for a task item; `None` for an item without a box.
    pub checked: Option<bool>,
    /// Its text, after its marker and box.
    pub text: &'a str,
    /// Where its box is in the line, in bytes, or where one would go after its marker.
    at: usize,
}

/// A fence opening or closing a code block on `line`: up to three spaces, then three or more backticks or tildes.
fn fence(line: &str) -> Option<(char, usize)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let marker = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.len() - rest.trim_start_matches(marker).len();
    (run >= 3).then_some((marker, run))
}

/// Whether `line` is a thematic break, three or more of one of `-`, `*` and `_`, with spaces between.
fn rule(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|mark| marks.iter().all(|c| c == mark))
}

/// The item `line` starts, if it starts one: its indentation, the length of its marker, its box, and where that is.
fn item(line: &str) -> Option<(usize, Option<bool>, usize, &str)> {
    if rule(line) {
        return None;
    }
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = line.len() - rest.len();
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let marker = match rest.as_bytes().first()? {
        b'-' | b'*' | b'+' => 1,
        _ if (1..=9).contains(&digits)
            && matches!(rest.as_bytes().get(digits), Some(b'.' | b')')) =>
        {
            digits + 1
        }
        _ => return None,
    };
    let after = &rest[marker..];
    if !(after.is_empty() || after.starts_with([' ', '\t'])) {
        return None;
    }
    let text = after.trim_start_matches([' ', '\t']);
    let at = line.len() - text.len();
    let (checked, text) = match text.get(..3) {
        Some("[ ]") => (Some(false), &text[3..]),
        Some("[x]" | "[X]") => (Some(true), &text[3..]),
        _ => (None, text),
    };
    // A box is followed by a space, or ends the line, as GitHub reads it.
    if checked.is_some() && !(text.is_empty() || text.starts_with([' ', '\t'])) {
        return Some((indent, None, at, after.trim_start_matches([' ', '\t'])));
    }
    Some((indent, checked, at, text.trim()))
}

/// The list items of `body`, in order.
pub fn items(body: &str) -> Vec<Item<'_>> {
    let mut items = Vec::new();
    let mut fenced: Option<(char, usize)> = None;
    // The indentation of each item that encloses the next.
    let mut open: Vec<usize> = Vec::new();
    for (index, line) in body.lines().enumerate() {
        if let Some((marker, run)) = fenced {
            if fence(line).is_some_and(|(closing, length)| closing == marker && length >= run) {
                fenced = None;
            }
            continue;
        }
        if let Some(opening) = fence(line) {
            fenced = Some(opening);
            continue;
        }
        match item(line) {
            Some((indent, checked, at, text)) => {
                while open.last().is_some_and(|enclosing| *enclosing >= indent) {
                    open.pop();
                }
                open.push(indent);
                items.push(Item {
                    line: index,
                    depth: open.len(),
                    checked,
                    text,
                    at,
                });
            }
            // A line that is not indented, and not blank, ends the lists before it.
            None if !line.trim().is_empty() && !line.starts_with([' ', '\t']) => open.clear(),
            None => {}
        }
    }
    items
}

/// `body` with the item on `line` ticked, or unticked, as `checked` says, given a box if it has none; or `None` when no
/// item is on that line.
pub fn set(body: &str, line: usize, checked: bool) -> Option<String> {
    let found = items(body).into_iter().find(|item| item.line == line)?;
    let mark = if checked { "[x]" } else { "[ ]" };
    let lines: Vec<String> = body
        .lines()
        .enumerate()
        .map(|(index, text)| {
            if index != line {
                return text.to_owned();
            }
            let (before, after) = text.split_at(found.at);
            match found.checked {
                Some(_) => format!("{before}{mark}{}", &after[3..]),
                None if after.is_empty() => format!("{} {mark}", before.trim_end()),
                None => format!("{before}{mark} {after}"),
            }
        })
        .collect();
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(body: &str) -> Vec<(usize, usize, Option<bool>, &str)> {
        items(body)
            .into_iter()
            .map(|item| (item.line, item.depth, item.checked, item.text))
            .collect()
    }

    #[test]
    fn items_are_found_with_their_boxes_and_depth_but_not_in_code_or_rules() {
        let body = "Agenda:\n\n- [ ] Pairing window\n- [x] Panels\n  * [X] Nested done\n  + plain nested\n1. Numbered\n2) [ ] Numbered task\n\n```\n- [ ] in code\n```\n- - -\n-not an item\n- [x]glued\n-\n- [ ]";
        assert_eq!(
            summary(body),
            [
                (2, 1, Some(false), "Pairing window"),
                (3, 1, Some(true), "Panels"),
                (4, 2, Some(true), "Nested done"),
                (5, 2, None, "plain nested"),
                (6, 1, None, "Numbered"),
                (7, 1, Some(false), "Numbered task"),
                (14, 1, None, "[x]glued"),
                (15, 1, None, ""),
                (16, 1, Some(false), ""),
            ]
        );
        // A fence indented as code opens no block, and digits that are not a number's marker start no item.
        assert_eq!(
            summary("    ```\n- [ ] after\n2026 was a year\n12x\n"),
            [(1, 1, Some(false), "after")]
        );
        // A paragraph that is not indented ends the list, so an item after it starts again at the top.
        assert_eq!(summary("- a\n  - b\nText\n  - c")[2].1, 1);
        assert!(items("").is_empty());
    }

    #[test]
    fn an_item_is_ticked_or_unticked_in_place_and_given_a_box_if_it_has_none() {
        let body = "- [ ] One\n  - [x] Two\n- Three\n-\n1. Four\nText";
        assert_eq!(
            set(body, 0, true).unwrap(),
            "- [x] One\n  - [x] Two\n- Three\n-\n1. Four\nText"
        );
        assert_eq!(
            set(body, 1, false).unwrap(),
            "- [ ] One\n  - [ ] Two\n- Three\n-\n1. Four\nText"
        );
        assert_eq!(
            set(body, 2, true).unwrap().lines().nth(2),
            Some("- [x] Three")
        );
        assert_eq!(set(body, 3, false).unwrap().lines().nth(3), Some("- [ ]"));
        assert_eq!(
            set(body, 4, true).unwrap().lines().nth(4),
            Some("1. [x] Four")
        );
        // Ticking what is already ticked changes nothing; a line that is no item is refused.
        assert_eq!(set(body, 1, true).unwrap(), body);
        assert_eq!(set(body, 5, true), None);
        assert_eq!(set(body, 99, true), None);
    }
}
