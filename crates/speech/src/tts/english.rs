// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! English phonemes for Kokoro's American voices, from misaki's rules and lexicons, which Kokoro was trained on, as `loqui-g2p` ports them to Rust, with its neural model for words they lack.
//!
//! Text is first normalized for what the agent writes and misaki reads poorly: times, ISO dates, units, Rust paths and generics, identifiers in snake or camel case, file names and paths, and web addresses. Words misaki gets wrong are given their phonemes: those in a dictionary of pronunciations, such as "README", and heteronyms that its tagger misreads, such as "live" in "go live".
use std::{path::Path, sync::LazyLock};

use loqui_g2p::{Dialect, G2p, OovFallback};
use regex::{Captures, Regex};

use super::{
    heteronyms,
    phonemes::Phonemizer,
    pronunciation::{self, Pronunciations},
};

/// American English, for `af_heart` and Kokoro's other `a` voices.
pub(crate) struct English {
    g2p: G2p,
    pronunciations: Pronunciations,
}

impl English {
    /// Parse the lexicons and load the model for unknown words, embedded in `loqui-g2p`, which takes about a tenth of a second, with Tomori's own pronunciations.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_pronunciations(None)
    }

    /// As [`new`](Self::new), adding and preferring the pronunciations in `file`, if it is given and exists.
    pub(crate) fn with_pronunciations(file: Option<&Path>) -> Self {
        // The data is part of the crate, so it fails only if the crate itself is broken.
        let g2p = G2p::new(Dialect::American, OovFallback::Neural)
            .expect("loqui-g2p's embedded lexicons and model load");
        let pronunciations = Pronunciations::load(file, |respelling| phonemes(&g2p, respelling));
        Self {
            g2p,
            pronunciations,
        }
    }

    /// `text` as it is given to misaki: normalized, with markup giving the phonemes of each word that misaki would get wrong.
    fn marked(&self, text: &str) -> String {
        let (text, marks) = self.pronunciations.protect(text);
        let text = heteronyms::mark(&normalize(&text));
        pronunciation::restore(&text, &marks)
    }
}

/// The phonemes of `text`, a few words, as `g2p` reads them, with a space between words.
fn phonemes(g2p: &G2p, text: &str) -> String {
    let mut phonemes = String::new();
    for token in g2p.tokens(text) {
        phonemes.push_str(&token.phonemes);
        if token.whitespace {
            phonemes.push(' ');
        }
    }
    phonemes.trim().to_owned()
}

impl Phonemizer for English {
    fn sentences(&self, text: &str) -> Vec<String> {
        let mut sentences = Vec::new();
        let mut sentence = String::new();
        for token in self.g2p.tokens(&self.marked(text)) {
            sentence.push_str(&token.phonemes);
            // A silent token, such as an emoji, leaves one space rather than two.
            if token.whitespace && !sentence.is_empty() && !sentence.ends_with(' ') {
                sentence.push(' ');
            }
            // Sentences end where sherpa-onnx 1.13 ended them.
            if token.phonemes.ends_with(['.', '!', '?', ';']) {
                sentences.push(std::mem::take(&mut sentence));
            }
        }
        sentences.push(sentence);
        sentences
            .into_iter()
            .map(|sentence| sentence.trim().to_owned())
            .filter(|sentence| !sentence.is_empty())
            .collect()
    }
}

/// A regular expression written in this file.
fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("a valid pattern")
}

/// Replace every match of `regex` in `text` with what `replace` says, again until nothing changes, for matches that overlap, such as each slash of `a/b/c`.
fn replace_all(text: String, regex: &Regex, replace: impl Fn(&Captures) -> String) -> String {
    let mut text = text;
    loop {
        let replaced = regex.replace_all(&text, &replace).into_owned();
        if replaced == text {
            return text;
        }
        text = replaced;
    }
}

/// `text` as misaki reads it best.
pub(crate) fn normalize(text: &str) -> String {
    // Dates and times are read from ASCII digits only, as `\d` is any script's digit, which `parse` refuses.
    static ADDRESS: LazyLock<Regex> = LazyLock::new(|| pattern(r"\bhttps?://"));
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| pattern(r"\b([0-9]{4})-([0-9]{2})-([0-9]{2})\b"));
    static TIME: LazyLock<Regex> =
        LazyLock::new(|| pattern(r"\b([0-9]+(?::[0-9]+)+)\b(\s*(?i:[ap]\.?m)\b)?"));
    static GENERIC: LazyLock<Regex> = LazyLock::new(|| pattern(r"(\w)<([\w ,&']+)>"));
    static UNIT: LazyLock<Regex> = LazyLock::new(|| {
        pattern(r"\b(\d+(?:\.\d+)?) ?(ms|µs|μs|ns|Hz|kHz|MHz|GHz|kB|KB|MB|GB|TB|KiB|MiB|GiB|TiB)\b")
    });
    static NUMBER: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b\d+(?:[.,]\d+)*\b"));
    static CAMEL: LazyLock<Regex> = LazyLock::new(|| pattern(r"([a-z])([A-Z][a-z])"));
    // A placeholder for a word with a pronunciation, `\x{E000}` to `\x{F8FF}`, counts as a word here.
    static DOT: LazyLock<Regex> = LazyLock::new(|| {
        pattern(r"([[:alpha:]]{2}|[\x{E000}-\x{F8FF}])\.([[:alpha:]]{2}|[\x{E000}-\x{F8FF}])")
    });
    static SLASH: LazyLock<Regex> =
        LazyLock::new(|| pattern(r"([[:alnum:]\x{E000}-\x{F8FF}])/([[:alnum:]\x{E000}-\x{F8FF}])"));

    let text = ADDRESS.replace_all(text, "").into_owned();
    let text = replace_all(text, &DATE, date);
    let text = replace_all(text, &TIME, time);
    // `std::sync::mpsc` as "std sync mpsc", `speak()` as "speak", and `max_tokens` as "max tokens".
    let text = text.replace("::", " ").replace("()", "").replace('_', " ");
    // `Vec<u8>` as "Vec of u8".
    let text = replace_all(text, &GENERIC, |captures| {
        format!("{} of {}", &captures[1], &captures[2])
    });
    let text = replace_all(text, &UNIT, unit);
    // misaki reads every number of four digits as a year, such as port `8443` as "eighty-four forty-three", so those that cannot be years are given a thousands separator, and read as "eight thousand four hundred forty-three".
    let text = replace_all(text, &NUMBER, |captures| {
        let number = &captures[0];
        match number.parse::<u32>() {
            Ok(2100..) if number.len() == 4 => format!("{},{}", &number[..1], &number[1..]),
            _ => number.to_owned(),
        }
    });
    // `KokoroSynthesizer` as "Kokoro Synthesizer", but `macOS` as it is.
    let text = replace_all(text, &CAMEL, |captures| {
        format!("{} {}", &captures[1], &captures[2])
    });
    // `kokoro.rs` as "kokoro dot rs", but not `e.g.` or `a.m.`.
    let text = replace_all(text, &DOT, |captures| {
        format!("{} dot {}", &captures[1], &captures[2])
    });
    // `src/tts` as "src slash tts", but `and/or` as "and or", and `50/50` as it is.
    let text = text.replace("and/or", "and or");
    let text = replace_all(text, &SLASH, |captures| {
        let (before, after) = (&captures[1], &captures[2]);
        if before
            .chars()
            .chain(after.chars())
            .all(|c| c.is_ascii_digit())
        {
            format!("{before}/{after}")
        } else {
            format!("{before} slash {after}")
        }
    });
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// An ISO date, such as `2026-11-10`, as "November 10th, 2026"; anything else that looks like one, as it is.
fn date(captures: &Captures) -> String {
    let (year, month, day) = (&captures[1], &captures[2], &captures[3]);
    let month: usize = month.parse().expect("two digits");
    let day: u32 = day.parse().expect("two digits");
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return captures[0].to_owned();
    }
    let suffix = match (day % 10, day / 10) {
        (1, tens) if tens != 1 => "st",
        (2, tens) if tens != 1 => "nd",
        (3, tens) if tens != 1 => "rd",
        _ => "th",
    };
    format!("{} {day}{suffix}, {year}", MONTHS[month - 1])
}

/// A time, such as `3:45`, as "3 45", `3:05` as "3 oh 5", `3:00` as "3 o'clock", or "3" before "pm", and `14:00` as "14 hundred"; other numbers joined by colons, such as `16:9`, with spaces.
fn time(captures: &Captures) -> String {
    let parts: Vec<&str> = captures[1].split(':').collect();
    let meridiem = captures.get(2).map_or("", |meridiem| meridiem.as_str());
    let spoken = match parts[..] {
        [hour, minute] if hour.len() <= 2 && minute.len() == 2 => {
            let hours: u32 = hour.parse().expect("digits");
            let minutes: u32 = minute.parse().expect("digits");
            match (hours, minutes) {
                (25.., _) | (_, 60..) => parts.join(" "),
                (_, 0) if !meridiem.is_empty() => hour.to_owned(),
                (1..=12, 0) => format!("{hour} o'clock"),
                (_, 0) => format!("{hour} hundred"),
                (_, 1..=9) => format!("{hour} oh {}", &minute[1..]),
                _ => format!("{hour} {minute}"),
            }
        }
        _ => parts.join(" "),
    };
    format!("{spoken}{meridiem}")
}

/// A number and a unit of time, frequency or size, such as `180 ms`, with the unit's name.
fn unit(captures: &Captures) -> String {
    let (number, unit) = (&captures[1], &captures[2]);
    let name = match unit {
        "ms" => "millisecond",
        "µs" | "μs" => "microsecond",
        "ns" => "nanosecond",
        "Hz" => "hertz",
        "kHz" => "kilohertz",
        "MHz" => "megahertz",
        "GHz" => "gigahertz",
        "kB" | "KB" => "kilobyte",
        "MB" => "megabyte",
        "GB" => "gigabyte",
        "TB" => "terabyte",
        "KiB" => "kibibyte",
        "MiB" => "mebibyte",
        "GiB" => "gibibyte",
        _ => "tebibyte",
    };
    let plural = if number == "1" || name.ends_with("hertz") {
        ""
    } else {
        "s"
    };
    format!("{number} {name}{plural}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_what_the_agent_writes() {
        for (text, expected) in [
            // Times.
            ("At 3:45 pm.", "At 3 45 pm."),
            ("It's 10:30 now.", "It's 10 30 now."),
            (
                "At 3:05, 3:00 and 3:00 a.m.",
                "At 3 oh 5, 3 o'clock and 3 a.m.",
            ),
            ("At 14:00 and 0:00.", "At 14 hundred and 0 hundred."),
            (
                "At 12:30:15, 16:9, 25:00 and 3:75.",
                "At 12 30 15, 16 9, 25 00 and 3 75.",
            ),
            // ISO dates.
            ("On 2026-11-10.", "On November 10th, 2026."),
            (
                "2026-01-01, 2026-02-02, 2026-03-03, 2026-04-11, 2026-05-12, 2026-06-13, 2026-07-21, 2026-08-22, 2026-09-23 and 2026-10-31.",
                "January 1st, 2026, February 2nd, 2026, March 3rd, 2026, April 11th, 2026, May 12th, 2026, June 13th, 2026, July 21st, 2026, August 22nd, 2026, September 23rd, 2026 and October 31st, 2026.",
            ),
            (
                "Build 2026-13-01 and 2026-12-00.",
                "Build 2026-13-01 and 2026-12-00.",
            ),
            // Units.
            (
                "180 ms, 1 ms and 2.5MB.",
                "180 milliseconds, 1 millisecond and 2.5 megabytes.",
            ),
            (
                "5 µs, 6 μs, 7 ns, 90 Hz, 1 kHz, 2 MHz, 3 GHz.",
                "5 microseconds, 6 microseconds, 7 nanoseconds, 90 hertz, 1 kilohertz, 2 megahertz, 3 gigahertz.",
            ),
            (
                "1 kB, 2 KB, 3 GB, 4 TB, 5 KiB, 6 MiB, 7 GiB, 8 TiB.",
                "1 kilobyte, 2 kilobytes, 3 gigabytes, 4 terabytes, 5 kibibytes, 6 mebibytes, 7 gibibytes, 8 tebibytes.",
            ),
            (
                "The 1990s, x86 MB and 5 ms.",
                "The 1990s, x86 MB and 5 milliseconds.",
            ),
            // Numbers of four digits that cannot be years.
            (
                "Port 8443, 4096 MB, 2100, 3.1415 and 1,234 in 2026, 1999 and 2099.",
                "Port 8,443, 4,096 megabytes, 2,100, 3.1415 and 1,234 in 2026, 1999 and 2099.",
            ),
            (
                "Versions 2.0.1 and 0042, and 65536 or 2100s.",
                "Versions 2.0.1 and 0042, and 65536 or 2100s.",
            ),
            // Code.
            ("It uses std::sync::mpsc.", "It uses std sync mpsc."),
            (
                "Call speak() with max_tokens.",
                "Call speak with max tokens.",
            ),
            (
                "A Vec<u8> and Vec<Option<String>>, but a < b > c.",
                "A Vec of u8 and Vec of Option of String, but a < b > c.",
            ),
            (
                "KokoroSynthesizer, OpenCode, macOS and iOS.",
                "Kokoro Synthesizer, Open Code, macOS and iOS.",
            ),
            // Files and addresses.
            (
                "Edit src/tts/kokoro.rs and Cargo.toml.",
                "Edit src slash tts slash kokoro dot rs and Cargo dot toml.",
            ),
            (
                "See https://github.com/ioquatix/tomori.",
                "See github dot com slash ioquatix slash tomori.",
            ),
            (
                "Use e.g. a.m. and/or 50/50 or 1/2.",
                "Use e.g. a.m. and or 50/50 or 1/2.",
            ),
            // Digits of other scripts are not read as dates or times.
            (
                "At ٣:٤٥, 3:٤ and １２:３４ on ٢٠٢٦-١١-١٠ or 2026-١١-10.",
                "At ٣:٤٥, 3:٤ and １２:３４ on ٢٠٢٦-١١-١٠ or 2026-١١-10.",
            ),
            // Whitespace.
            ("  Hello\n   there.  ", "Hello there."),
        ] {
            assert_eq!(normalize(text), expected, "{text}");
        }
    }

    /// The phonemes of `text`, one string per sentence.
    fn sentences(text: &str) -> Vec<String> {
        static ENGLISH: LazyLock<English> = LazyLock::new(English::new);
        ENGLISH.sentences(text)
    }

    #[test]
    fn pronounces_words_from_the_lexicon() {
        assert_eq!(sentences("Hello world."), ["həlˈO wˈɜɹld."]);
        // A heteronym, by its part of speech.
        assert_eq!(
            sentences("Read the design; I read it yesterday."),
            ["ɹˈid ðə dəzˈIn;", "ˌI ɹˈɛd ɪt jˈɛstəɹdˌA."]
        );
    }

    #[test]
    fn pronounces_words_the_lexicon_lacks() {
        // Names that misaki's lexicons do not have are pronounced by the neural model, not dropped.
        let [tomori] = &sentences("Tomori")[..] else {
            unreachable!("one sentence");
        };
        assert!(!tomori.is_empty());
        assert!(!tomori.contains('❓'), "{tomori}");
        assert_eq!(sentences("Hinoki"), ["hɪnˈOki"]);
    }

    #[test]
    fn reads_times_without_a_colon() {
        let spoken = sentences("The build finished at 3:45 pm.").join(" ");
        assert!(!spoken.contains(':'), "{spoken}");
        assert_eq!(spoken, "ðə bˈɪld fˈɪnəʃt æt θɹˈi fˈɔɹTi fˈIv pˌiˈɛm.");
    }

    /// The phonemes of each word of `text` that is `word`, in any case, in order.
    fn pronounced(text: &str, word: &str) -> Vec<String> {
        static ENGLISH: LazyLock<English> = LazyLock::new(English::new);
        ENGLISH
            .g2p
            .tokens(&ENGLISH.marked(text))
            .into_iter()
            .filter(|token| token.text.eq_ignore_ascii_case(word))
            .map(|token| token.phonemes)
            .collect()
    }

    #[test]
    fn pronounces_the_listening_tests_words() {
        // The sentences of the maintainer's listening test of the phonemizer, in November 2026.
        let text = "The live preview lives in this folder.";
        assert_eq!(pronounced(text, "live"), ["lˈIv"]);
        assert_eq!(pronounced(text, "lives"), ["lˈɪvz"]);
        assert_eq!(
            sentences("Publish the crate to crates.io when you're ready."),
            ["pˈʌblɪʃ ðə kɹˈAt tə kɹˈAts dˈɑt ˈIˌO wˌɛn jʊɹ ɹˈɛdi."]
        );
        assert_eq!(
            pronounced("We'll go live with it tomorrow, and I live nearby.", "live"),
            ["lˈIv", "lˈɪv"]
        );
        let text = "Check the live log on crates.io before it goes live.";
        assert_eq!(pronounced(text, "live"), ["lˈIv", "lˈIv"]);
        assert!(sentences(text)[0].contains("kɹˈAts dˈɑt ˈIˌO"));
        let text = "Read the README, then read it again: I read it yesterday.";
        assert_eq!(pronounced(text, "read"), ["ɹˈid", "ɹˈid", "ɹˈɛd"]);
        assert_eq!(pronounced(text, "README"), ["ɹˈidmi"]);
        assert_eq!(
            sentences(text),
            ["ɹˈid ðə ɹˈidmi, ðˈɛn ɹˈid ɪt əɡˈɛn: ˌI ɹˈɛd ɪt jˈɛstəɹdˌA."]
        );
    }

    #[test]
    fn pronounces_live_by_the_words_around_it() {
        for (text, expected) in [
            ("It goes live!", &["lˈIv"][..]),
            ("We're live, and it is live.", &["lˈIv", "lˈIv"]),
            ("Make it live; is it live?", &["lˈIv", "lˈIv"]),
            (
                "Start the live stream, live audio and live-reload.",
                &["lˈIv", "lˈIv", "lˈIv"],
            ),
            ("I want to live here, where we live.", &["lˈɪv", "lˈɪv"]),
            // Left to the tagger.
            ("Long live the king. Let's live.", &["lˈɪv", "lˈɪv"]),
        ] {
            assert_eq!(pronounced(text, "live"), expected, "{text}");
        }
        assert_eq!(
            pronounced("She lives nearby. He lived there.", "lives"),
            ["lˈɪvz"]
        );
        assert_eq!(
            pronounced("She lives nearby. He lived there.", "lived"),
            ["lˈɪvd"]
        );
    }

    #[test]
    fn pronounces_imperatives_that_start_a_sentence_as_verbs() {
        assert_eq!(sentences("Use JSON."), ["jˈuz ʤˈAsᵊn."]);
        assert_eq!(
            pronounced("Record the audio. Press record.", "record"),
            ["ɹəkˈɔɹd", "ɹˈɛkəɹd"]
        );
        assert_eq!(
            pronounced("Present your findings.", "present"),
            ["pɹizˈɛnt"]
        );
    }

    #[test]
    fn pronounces_words_with_a_pronunciation_in_any_case_and_in_code() {
        for word in ["README", "Readme", "readme"] {
            assert_eq!(
                pronounced(&format!("The {word} is here."), word),
                ["ɹˈidmi"]
            );
        }
        assert_eq!(
            sentences("Open `README.md` (README)."),
            ["ˈOpᵊn ɹˈidmi dˈɑt ˌɛmdˈi (ɹˈidmi)."]
        );
        assert_eq!(
            sentences("See GitHub and github."),
            ["sˈi ɡˈɪt hˈʌb ænd ɡˈɪt hˈʌb."]
        );
        assert_eq!(
            sentences("It uses sherpa-onnx and Kokoro."),
            ["ˌɪt jˈuzᵻz ʃˈɜɹpə ˈɑnɪks ænd kˈOkəɹO."]
        );
    }

    #[test]
    fn pronounces_id_as_its_letters() {
        // The maintainer heard "id" said as the Freudian id, in November 2026.
        for word in ["ID", "Id", "id"] {
            assert_eq!(pronounced(&format!("Its {word} is 3."), word), ["ˌIdˈi"]);
        }
        for word in ["IDs", "ids"] {
            assert_eq!(pronounced(&format!("The {word} match."), word), ["ˌIdˈiz"]);
        }
        // A panel's header line, code, punctuation and identifiers.
        assert_eq!(sentences("id: 42"), ["ˌIdˈi: fˈɔɹTi tˈu"]);
        assert_eq!(
            sentences("Read the `id` and `ids` fields."),
            ["ɹˈid ði ˌIdˈi ænd ˌIdˈiz fˈildz."]
        );
        assert_eq!(
            sentences("(id), \"id\", panel_id and session-id."),
            ["(ˌIdˈi), ”ˌIdˈi, pˈænᵊl ˌIdˈi ænd sˈɛʃənˌIdˈi."]
        );
        assert_eq!(pronounced("The panel's id's text.", "id's"), ["ˌIdˈiz"]);
        // Not within a word.
        assert_eq!(
            sentences("I did have an idea, and it is valid."),
            ["ˌI dˈɪd hæv ɐn Idˈiə, ænd ɪt ɪz vˈælɪd."]
        );
        // Its tokenizer reads "id" as "I'd", and "Id." as an abbreviation, which lost the full stop that ends a sentence.
        assert_eq!(
            sentences("Point at it and I'll inspect its id. Then its Id."),
            ["pˈYnt æt ɪt ænd ˌIl ɪnspˈɛkt ɪts ˌIdˈi.", "ðˈɛn ɪts ˌIdˈi."]
        );
    }

    #[test]
    fn pronounces_acronyms_as_their_letters_in_any_case() {
        for (word, expected) in [
            ("adb", "ˌAdˌibˈi"),
            ("apk", "ˌApˌikˈA"),
            ("apks", "ˌApˌikˈAz"),
            ("api", "ˌApˌiˈI"),
            ("cli", "sˌiˌɛlˈI"),
            ("cpu", "sˌipˌijˈu"),
            ("gpu", "ʤˌipˌijˈu"),
            ("os", "ˌOˈɛs"),
            ("prs", "pˌiˈɑɹz"),
            ("ui", "jˌuˈI"),
            ("url", "jˌuˌɑɹˈɛl"),
            ("urls", "jˌuˌɑɹˈɛlz"),
            ("usb", "jˌuˌɛsbˈi"),
            ("openxr", "ˌOpᵊnˌɛksˈɑɹ"),
            ("promql", "pɹˌɑmkjˌuˈɛl"),
        ] {
            for written in [word.to_owned(), word.to_uppercase()] {
                assert_eq!(
                    pronounced(&format!("The {written} works."), &written),
                    [expected],
                    "{written}"
                );
            }
        }
        assert_eq!(pronounced("Run git rerere.", "rerere"), ["ɹˌiɹˌiɹˈi"]);
        // As `loqui-g2p` already says them, with no pronunciation.
        assert_eq!(
            sentences("The IDs are on the APK's MCP panel, and the PRs pass CI."),
            ["ði ˌIdˈiz ɑɹ ˌɔn ði ˌApˌikˈAz ˌɛmsˌipˈi pˈænᵊl, ænd ðə pˌiˈɑɹz pˈæs sˌiˈI."]
        );
    }

    #[test]
    fn the_built_in_pronunciations_are_needed_and_right() {
        let without = G2p::new(Dialect::American, OovFallback::Neural).unwrap();
        let english = English::new();
        let document: toml_edit::DocumentMut = pronunciation::BUILT_IN.parse().unwrap();
        // None is left out.
        assert_eq!(english.pronunciations.len(), document.len());
        for (word, _) in document.iter() {
            let given = phonemes(&english.g2p, &english.marked(word));
            assert!(!given.is_empty(), "{word}");
            // `loqui-g2p` reads it differently without it, as it is written in some case.
            let capitalized = format!("{}{}", word[..1].to_uppercase(), &word[1..]);
            assert!(
                [word.to_owned(), word.to_uppercase(), capitalized]
                    .iter()
                    .any(|written| phonemes(&without, written) != given),
                "{word} is pronounced {given} without it"
            );
        }
    }

    #[test]
    fn a_servers_own_pronunciations_are_preferred() {
        let file = std::env::temp_dir().join(format!(
            "tomori-english-pronunciations-{}.toml",
            std::process::id()
        ));
        std::fs::write(&file, "readme = \"read it\"\nhinoki = \"/hˈinˌOki/\"\n").unwrap();
        let english = English::with_pronunciations(Some(&file));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            english.sentences("The README, by Hinoki."),
            ["ðə ɹˈid ɪt, bI hˈinˌOki."]
        );
    }

    #[test]
    fn phonemizes_any_text_without_failing() {
        // A panic would stop synthesis until the server restarts, so text the agent might write, however odd, is phonemized, in a bounded time and to bounded phonemes.
        let long = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo0123456789+/".repeat(40);
        for text in [
            "",
            "👨\u{200d}👩\u{200d}👧\u{200d}👦",
            "日本語のテキストです。 Привет, мир! مرحبا",
            "٢٠٢٦-١١-١٠ ١٢:٣٤ 3:٤ 2026-١١-10 １２:３４",
            "99:99:99:99 12:30pm 1234567890123456789012345678901234567890",
            "fn main() { let x: Vec<Vec<u8>> = vec![1_000, 0x1F]; }",
            "[foo](/bar/) [x](/) [](/a/) ](/ \u{E000}\u{200B}a\u{0301}",
            "https://example.com/a/b/c?x=1&y=2#frag",
            &long,
        ] {
            let started = std::time::Instant::now();
            let spoken: usize = sentences(text).iter().map(|s| s.chars().count()).sum();
            assert!(spoken <= 4 * text.chars().count() + 64, "{text}: {spoken}");
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "{text}"
            );
        }
    }

    #[test]
    fn divides_sentences_where_they_end() {
        assert_eq!(
            sentences("Done 🎉 — really? Yes! It is. Then"),
            ["dˈʌn — ɹˈiᵊli?", "jˈɛs!", "ˌɪt ɪz.", "ðˈɛn"]
        );
        assert!(sentences("🎉").is_empty());
    }
}
