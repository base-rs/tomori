// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Phonemes for a voice model: the seam at which a language pack (#103) will supply its own phonemizer.
use std::collections::HashSet;

/// The most phonemes Kokoro is given at once. Its context is 510 tokens, and sherpa-onnx cuts a longer input after 400, wherever that falls, so a sentence longer than this is split at a word break first.
pub(crate) const MAX_PHONEMES: usize = 400;

/// A sentence of at most this many phonemes is spoken with the one before it, as sherpa-onnx 1.13 joined them, so that a short sentence is not given a pause of its own.
const SHORT: usize = 10;

/// A short sentence joins the one before it when they have fewer than this many phonemes together.
const TOGETHER: usize = 50;

/// A sentence of fewer than this many phonemes always joins the one before it.
const TINY: usize = 5;

/// Turns text into the phonemes a voice reads, in sentences. Each language's voice needs its own: English is [`super::english::English`].
pub(crate) trait Phonemizer: Send {
    /// The phonemes of `text`, a sentence at a time, each with its closing punctuation, which the voice uses for prosody. A sentence with nothing to pronounce, such as an emoji, is left out.
    fn sentences(&self, text: &str) -> Vec<String>;
}

/// The phonemes a voice model knows, by the symbols listed in its `tokens.txt`, one per line, with its id after the last space.
pub(crate) fn symbols(tokens: &str) -> HashSet<char> {
    tokens
        .lines()
        .filter_map(|line| line.rsplit_once(' '))
        .filter_map(|(symbol, _)| {
            let mut characters = symbol.chars();
            match (characters.next(), characters.next()) {
                (Some(symbol), None) => Some(symbol),
                _ => None,
            }
        })
        .collect()
}

/// The code points Kokoro is given for `sentences`: each sentence as it is spoken, without symbols the model does not know, short sentences joined to the one before them and long ones split at a word break, so that each is at most [`MAX_PHONEMES`].
pub(crate) fn codepoints(sentences: Vec<String>, known: &HashSet<char>) -> Vec<Vec<i32>> {
    let mut spoken: Vec<Vec<char>> = Vec::new();
    for sentence in sentences {
        let mut phonemes: Vec<char> = sentence
            .chars()
            .filter(|symbol| {
                let kept = known.contains(symbol);
                if !kept {
                    log::debug!("Kokoro has no phoneme {symbol:?}; it is left out.");
                }
                kept
            })
            .collect();
        trim(&mut phonemes);
        if phonemes.is_empty() {
            continue;
        }
        match spoken.last_mut() {
            Some(last)
                if phonemes.len() < TINY
                    || (phonemes.len() <= SHORT && last.len() + phonemes.len() < TOGETHER) =>
            {
                last.push(' ');
                last.extend(phonemes);
            }
            _ => spoken.push(phonemes),
        }
    }
    spoken
        .into_iter()
        .flat_map(split)
        .map(|phonemes| phonemes.into_iter().map(|symbol| symbol as i32).collect())
        .collect()
}

/// Remove spaces at either end.
fn trim(phonemes: &mut Vec<char>) {
    while phonemes.last() == Some(&' ') {
        phonemes.pop();
    }
    let leading = phonemes.iter().take_while(|&&symbol| symbol == ' ').count();
    phonemes.drain(..leading);
}

/// Split a sentence into parts of at most [`MAX_PHONEMES`]: after the last clause break, a comma, semicolon, colon or dash followed by a space, within the limit, or else the last space, or else at the limit.
fn split(mut phonemes: Vec<char>) -> Vec<Vec<char>> {
    let mut parts = Vec::new();
    while phonemes.len() > MAX_PHONEMES {
        let window = &phonemes[..=MAX_PHONEMES];
        let breaks = |clause: bool| {
            (1..window.len()).rev().find(|&index| {
                window[index] == ' '
                    && (!clause || matches!(window[index - 1], ',' | ';' | ':' | '—'))
            })
        };
        let cut = breaks(true)
            .or_else(|| breaks(false))
            .unwrap_or(MAX_PHONEMES);
        let mut rest = phonemes.split_off(cut);
        trim(&mut rest);
        trim(&mut phonemes);
        parts.push(phonemes);
        phonemes = rest;
    }
    parts.push(phonemes);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> HashSet<char> {
        symbols(
            "; 1\n: 2\n, 3\n. 4\n  16\nA 24\nə 83\nh 50\nl 54\nO 31\nk 53\nˈ 156\n— 9\nmultiple 200\n",
        )
    }

    fn spoken(sentences: &[&str]) -> Vec<String> {
        codepoints(
            sentences
                .iter()
                .map(|&sentence| sentence.to_owned())
                .collect(),
            &known(),
        )
        .into_iter()
        .map(|codepoints| {
            codepoints
                .into_iter()
                .map(|codepoint| char::from_u32(codepoint as u32).unwrap())
                .collect()
        })
        .collect()
    }

    #[test]
    fn reads_the_symbols_of_a_voice() {
        let symbols = known();
        assert!(symbols.contains(&' '));
        assert!(symbols.contains(&'ə'));
        assert!(
            !symbols.contains(&'m'),
            "a symbol of several characters is not one phoneme"
        );
        assert_eq!(symbols.len(), 13);
    }

    #[test]
    fn leaves_out_unknown_symbols_and_empty_sentences() {
        // Each symbol left out is logged at debug level.
        log::set_max_level(log::LevelFilter::Trace);
        assert_eq!(spoken(&["həlˈO❓ həl.", "🎉", "  "]), ["həlˈO həl."]);
        assert!(spoken(&[]).is_empty());
    }

    #[test]
    fn joins_short_sentences_to_the_one_before() {
        let long = "həlˈO həlˈO həlˈO.";
        // The first sentence has nothing before it to join.
        assert_eq!(spoken(&["kA.", long]), ["kA.", long]);
        // A sentence of fewer than five phonemes always joins the one before it.
        assert_eq!(
            spoken(&[&long.repeat(3), "kA."]),
            [format!("{} kA.", long.repeat(3))]
        );
        // One of at most ten joins it while they are short together.
        assert_eq!(spoken(&[long, "həlˈO kA."]), [format!("{long} həlˈO kA.")]);
        assert_eq!(
            spoken(&[&long.repeat(3), "həlˈO kA."]),
            [long.repeat(3), "həlˈO kA.".to_owned()]
        );
        // A longer one is its own.
        assert_eq!(spoken(&[long, long]), [long, long]);
    }

    #[test]
    fn splits_long_sentences_at_a_clause_or_word_break() {
        let word = "həlˈO";
        let clause = format!("{},", [word; 10].join(" "));
        // At a clause break, within the limit.
        let sentence = [clause.as_str(); 10].join(" ");
        let parts = spoken(&[&sentence]);
        assert!(parts.len() > 1, "{parts:?}");
        assert!(
            parts
                .iter()
                .all(|part| part.chars().count() <= MAX_PHONEMES)
        );
        assert!(
            parts[..parts.len() - 1]
                .iter()
                .all(|part| part.ends_with(',')),
            "{parts:?}"
        );
        assert_eq!(parts.join(" "), sentence);
        // At a space, without clauses.
        let sentence = [word; 100].join(" ");
        let parts = spoken(&[&sentence]);
        assert!(parts.len() > 1);
        assert!(
            parts
                .iter()
                .all(|part| part.chars().count() <= MAX_PHONEMES)
        );
        assert_eq!(parts.join(" "), sentence);
        // At the limit, without spaces.
        let sentence = "k".repeat(MAX_PHONEMES * 2 + 1);
        let lengths: Vec<usize> = spoken(&[&sentence])
            .iter()
            .map(|part| part.chars().count())
            .collect();
        assert_eq!(lengths, [MAX_PHONEMES, MAX_PHONEMES, 1]);
    }
}
