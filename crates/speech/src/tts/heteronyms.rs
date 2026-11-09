// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Heteronyms that misaki's part-of-speech tagger reads wrongly in what the agent says, decided by the words next to them.
//!
//! The tagger reads "live" in "go live" and "goes live" as a verb, "lɪv", where it is "laɪv", and a verb that starts a sentence, such as "Use" in "Use JSON.", as a name, and so as a noun. Where a rule here decides, the phonemizer is told the word's phonemes with misaki's markup, `[live](/lˈIv/)`; elsewhere the tagger still decides, as it does correctly in "the live log", "I live nearby" and "the preview lives here".
use std::sync::LazyLock;

use regex::Regex;

use super::pronunciation::is_placeholder;

/// "live" as an adjective or adverb, and as a verb, in misaki's notation.
const LIVE: &str = "lˈIv";
const LIVE_VERB: &str = "lˈɪv";

/// Words after which "live" is an adjective or adverb: "go live", "is live", "the live log".
const BEFORE_LIVE: &[&str] = &[
    "go",
    "goes",
    "going",
    "went",
    "gone",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "am",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "stay",
    "stays",
    "stayed",
    "staying",
    "remain",
    "remains",
    "remained",
    "remaining",
    "the",
    "a",
    "an",
    "our",
    "your",
    "my",
    "its",
    "their",
];

/// Nouns that "live" describes: "live log", "live stream".
const LIVE_NOUNS: &[&str] = &[
    "audio",
    "broadcast",
    "broadcasts",
    "call",
    "calls",
    "captions",
    "chat",
    "coding",
    "data",
    "demo",
    "demos",
    "edit",
    "editing",
    "environment",
    "event",
    "events",
    "feed",
    "feeds",
    "log",
    "logs",
    "mode",
    "music",
    "output",
    "performance",
    "preview",
    "previews",
    "recording",
    "reload",
    "reloading",
    "server",
    "servers",
    "session",
    "sessions",
    "share",
    "sharing",
    "show",
    "site",
    "sites",
    "speech",
    "stream",
    "streaming",
    "streams",
    "test",
    "tests",
    "traffic",
    "transcript",
    "transcription",
    "update",
    "updates",
    "version",
    "video",
    "view",
];

/// Words after which "live" is a verb: "I live", "to live", "can live".
const BEFORE_VERB: &[&str] = &[
    "i",
    "we",
    "you",
    "they",
    "who",
    "to",
    "can",
    "could",
    "will",
    "would",
    "should",
    "might",
    "must",
    "may",
    "shall",
    "do",
    "does",
    "did",
    "don't",
    "doesn't",
    "didn't",
    "can't",
    "cannot",
    "won't",
    "wouldn't",
    "couldn't",
    "shouldn't",
];

/// Words that put "live" after a pronoun as an adjective: "make it live", "is it live?", "are you live?"
const MAKING: &[&str] = &[
    "make", "makes", "made", "making", "keep", "keeps", "kept", "keeping", "set", "sets",
    "setting", "am", "are", "is", "was", "were",
];

/// The pronouns of [`MAKING`].
const MADE: &[&str] = &[
    "i",
    "we",
    "you",
    "they",
    "it",
    "them",
    "this",
    "that",
    "everything",
];

/// Heteronyms that the tagger reads as names, and so as nouns, when they start a sentence as a verb, and their phonemes as verbs: "Use JSON.", "Record the audio.", "Present your findings."
const IMPERATIVES: &[(&str, &str)] = &[
    ("use", "jˈuz"),
    ("record", "ɹəkˈɔɹd"),
    ("present", "pɹizˈɛnt"),
];

/// Words that start an imperative's object: "Use the CLI.", "Present it."
const OBJECTS: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "it", "them", "your", "our", "my", "its",
    "their", "his", "her", "every", "all", "each", "some", "any", "both",
];

/// A word: letters, with an apostrophe and more letters, as in "it's", or a placeholder for a word with a pronunciation, such as "JSON".
static WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\p{L}+(?:['’]\p{L}+)?|[\x{E000}-\x{F8FF}]").expect("a valid pattern")
});

struct Word<'a> {
    start: usize,
    end: usize,
    text: &'a str,
    /// In lowercase, with a straight apostrophe.
    lower: String,
}

/// The words of `text`.
fn words(text: &str) -> Vec<Word<'_>> {
    WORD.find_iter(text)
        .map(|word| Word {
            start: word.start(),
            end: word.end(),
            text: word.as_str(),
            lower: word.as_str().to_lowercase().replace('’', "'"),
        })
        .collect()
}

/// `text` with misaki's markup for each heteronym that a rule here decides.
pub(crate) fn mark(text: &str) -> String {
    let words = words(text);
    let mut marked = String::with_capacity(text.len());
    let mut end = 0;
    for (index, word) in words.iter().enumerate() {
        if let Some(phonemes) = decide(text, &words, index) {
            marked.push_str(&text[end..word.start]);
            marked.push_str(&format!("[{}](/{phonemes}/)", word.text));
            end = word.end;
        }
    }
    marked.push_str(&text[end..]);
    marked
}

/// The phonemes of the word at `index` of `words`, in `text`, if a rule decides them.
fn decide(text: &str, words: &[Word], index: usize) -> Option<&'static str> {
    let word = &words[index];
    // The word before `index`, if only spaces come between them.
    let before = |index: usize| {
        let before = &words[index.checked_sub(1)?];
        text[before.end..words[index].start]
            .chars()
            .all(char::is_whitespace)
            .then_some(before.lower.as_str())
    };
    let after = words.get(index + 1).filter(|after| {
        let between = text[word.end..after.start].trim();
        between.is_empty() || between == "-"
    });
    let is = |word: Option<&str>, list: &[&str]| word.is_some_and(|word| list.contains(&word));
    let previous = before(index);
    if word.lower == "live" {
        let contracted = previous.is_some_and(|previous| {
            previous != "let's"
                && ["'s", "'re", "'m"]
                    .iter()
                    .any(|end| previous.ends_with(end))
        });
        if contracted || is(previous, BEFORE_LIVE) {
            return Some(LIVE);
        }
        if is(previous, MADE) && is(before(index - 1), MAKING) {
            return Some(LIVE);
        }
        // Before a noun it describes, or joined to one by a hyphen, as in "live-reload".
        if let Some(after) = after
            && (LIVE_NOUNS.contains(&after.lower.as_str()) || &text[word.end..after.start] == "-")
        {
            return Some(LIVE);
        }
        let verb = previous.is_some_and(|previous| {
            BEFORE_VERB.contains(&previous) || previous.ends_with("'ll") || previous.ends_with("'d")
        });
        return verb.then_some(LIVE_VERB);
    }
    let (_, verb) = IMPERATIVES
        .iter()
        .find(|(imperative, _)| *imperative == word.lower)?;
    // At the start of a sentence, or after "please", before its object, which may be a name, such as "Tomori", or a word with a pronunciation, such as "JSON".
    let starts = match index.checked_sub(1) {
        None => true,
        Some(before) => {
            text[words[before].end..word.start].contains(['.', '!', '?', ';', ':', '\n'])
        }
    };
    let object = after.is_some_and(|after| {
        OBJECTS.contains(&after.lower.as_str())
            || after
                .text
                .starts_with(|first: char| first.is_uppercase() || is_placeholder(first))
    });
    ((starts || previous == Some("please")) && object).then_some(verb)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heteronyms of `text` that a rule decides, as "live" for /laɪv/, "liv" for /lɪv/, and "VERB" for an imperative, and "?" for those left to the tagger.
    fn decided(text: &str) -> Vec<&'static str> {
        let words = words(text);
        (0..words.len())
            .filter(|&index| {
                ["live", "use", "record", "present"].contains(&words[index].lower.as_str())
            })
            .map(|index| match decide(text, &words, index) {
                Some(LIVE) => "live",
                Some(LIVE_VERB) => "liv",
                Some(_) => "VERB",
                None => "?",
            })
            .collect()
    }

    #[test]
    fn live_is_an_adjective_after_going_and_being() {
        for text in [
            "We'll go live with it tomorrow.",
            "Before it goes live.",
            "It is live now.",
            "The stream went live, and we are going live, as it has gone live.",
            "It's live. We're live! I'm live? Was it live: it was live.",
            "It has been live since Monday, and it stays live.",
            "Make it live, and keep them live.",
            "Are you live?",
        ] {
            let decisions = decided(text);
            assert!(
                decisions.iter().all(|&live| live == "live"),
                "{text}: {decisions:?}"
            );
        }
    }

    #[test]
    fn live_is_an_adjective_before_a_noun() {
        assert_eq!(
            decided(
                "Check the live log. Start live audio, live-reload and a live stream; we live stream on Fridays."
            ),
            ["live", "live", "live", "live", "live"]
        );
    }

    #[test]
    fn live_is_a_verb_after_a_subject() {
        assert_eq!(
            decided(
                "I live nearby, we live here, they'd live there, to live, who can live, and you'll live. Let's live."
            ),
            ["liv", "liv", "liv", "liv", "liv", "liv", "?"]
        );
    }

    #[test]
    fn other_lives_are_left_to_the_tagger() {
        assert_eq!(
            decided("Live. Long live the king, and the people, live!"),
            ["?", "?", "?"]
        );
    }

    #[test]
    fn imperatives_that_start_a_sentence_are_verbs() {
        assert_eq!(
            decided(
                "Use JSON. Record the audio! Present your findings; please use it. Present it. Use \u{E000}."
            ),
            ["VERB", "VERB", "VERB", "VERB", "VERB", "VERB"]
        );
        // As a noun, or within a sentence, they are left to the tagger.
        assert_eq!(
            decided("Use cases. The use of it. I present it. Record time, and a record. Use"),
            ["?", "?", "?", "?", "?", "?"]
        );
    }

    #[test]
    fn marks_what_it_decides() {
        assert_eq!(
            mark("Use JSON: it goes Live, and I live nearby."),
            "[Use](/jˈuz/) JSON: it goes [Live](/lˈIv/), and I [live](/lˈɪv/) nearby."
        );
        assert_eq!(mark("Nothing to mark."), "Nothing to mark.");
    }
}
