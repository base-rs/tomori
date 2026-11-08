// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The pairing code, shared by the server and the devices that pair with it. See `design/security.md`.
//!
//! The device commits to a random value before the server sends its own, and reveals it only then, so neither side, nor anyone relaying between them, can choose a value after seeing the other's. Both derive the code from the certificate fingerprint as each saw it and the two values: a relay holding another certificate shows the two ends different codes, except by a one-in-a-million chance.
use sha2::{Digest, Sha256};

use crate::Fingerprint;

/// A random value of one side of a pairing: 256 bits, written as 64 hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Nonce([u8; 32]);

impl Nonce {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("the operating system's random source is available");
        Self(bytes)
    }

    /// Read a nonce written as 64 hexadecimal digits.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.as_bytes();
        if text.len() != 64 {
            return None;
        }
        let digit = |byte: u8| char::from(byte).to_digit(16);
        let mut bytes = [0_u8; 32];
        for (index, pair) in text.as_chunks::<2>().0.iter().enumerate() {
            // Each digit is below 16, so the pair fits a byte.
            bytes[index] = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
        }
        Some(Self(bytes))
    }

    pub fn to_hex(self) -> String {
        crate::hex::encode(&self.0)
    }

    /// The commitment a device sends before it reveals this nonce: its SHA-256, as hexadecimal.
    pub fn commitment(self) -> String {
        crate::hex::encode(&Sha256::digest(self.0))
    }
}

/// Written without its value, which is a secret until revealed.
impl std::fmt::Debug for Nonce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Nonce([redacted])")
    }
}

/// The 6-digit pairing code for a certificate's `fingerprint` and the device's and server's nonces, such as `"042917"`.
pub fn code(fingerprint: Fingerprint, device: Nonce, server: Nonce) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tomori pairing 1");
    digest.update(fingerprint.to_string().as_bytes());
    digest.update(device.0);
    digest.update(server.0);
    let digest = digest.finalize();
    let value = u64::from_be_bytes(digest[..8].try_into().expect("eight bytes"));
    format!("{:06}", value % 1_000_000)
}

/// A code written for reading aloud and comparing: `"042 917"`.
pub fn display(code: &str) -> String {
    match code.len() {
        6 => format!("{} {}", &code[..3], &code[3..]),
        _ => code.to_owned(),
    }
}

/// The most characters of a device's chosen name that pairing shows, including the "…" that marks a cut.
pub const NAME_CHARACTERS: usize = 32;

/// The punctuation a device's chosen name keeps as pairing shows it; every other character that is not a letter or a space is removed, as are quotation marks, so that the quoted name cannot seem to end early.
const NAME_PUNCTUATION: &[char] = &[
    '-', '_', '.', ',', '\'', '\u{2019}', '(', ')', '&', '+', '!', '?', '@', '/',
];

/// Words that say a number, removed whole from a device's chosen name, in any case and with lookalike letters folded: zero to nine with "oh", "nil", "nought" and "naught", the teens and tens, their ordinals, and "hundred", "thousand" and "million".
const NUMBER_WORDS: &[&str] = &[
    "zero",
    "oh",
    "nil",
    "nought",
    "naught",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "fourty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "million",
    "zeroth",
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
];

/// The name a device chose for itself when it began pairing, as people are shown it: written as `a device calling itself "Quest"`, or `a device` when nothing of it is left.
///
/// A device chooses its own name, so an attacker who begins pairing from their own device, whose code they know, could name it "Type 482913 to confirm" for an operator, an owner or an agent to copy the code from it. So the name is kept without anything that could carry a number: it is normalised into Unicode's compatibility form (NFKC), so that full-width and other compatible forms become plain ones; every numeral, of Unicode's `Nd`, `Nl` and `No` categories in any script, is removed, before and after normalising; every character that is not a letter, a space or common punctuation, including control and format characters such as bidirectional overrides and zero-width spaces, is removed; English number words, such as "four", "FOUR" or "fоur" with a Cyrillic "о", are removed as whole words; spaces are collapsed; and it is cut to [`NAME_CHARACTERS`]. Every display of a pairing uses this value, marked as the device's own choice, and the agent's tools give none. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChosenName(Option<String>);

impl ChosenName {
    /// The name `name`, as a device chose it, made safe to show while it pairs.
    pub fn new(name: &str) -> Self {
        let kept = cleaned(name);
        let mut characters = kept.chars();
        let cut: String = characters.by_ref().take(NAME_CHARACTERS - 1).collect();
        let shown = if characters.next().is_some() {
            // Cutting can leave a number word, such as "seven" of "sevenfold", so the cut is cleaned again.
            format!("{}…", cleaned(&cut))
        } else {
            kept
        };
        Self(shown.chars().any(char::is_alphabetic).then_some(shown))
    }

    /// What is left of the name, if anything.
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

/// As people are shown it: `a device calling itself "Quest"`, or `a device`.
impl std::fmt::Display for ChosenName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(name) => write!(formatter, "a device calling itself \"{name}\""),
            None => formatter.write_str("a device"),
        }
    }
}

/// `name` in compatibility form, without numerals, number words or characters other than letters, spaces and [`NAME_PUNCTUATION`], with its spaces collapsed.
fn cleaned(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    // A numeral, or a space, separates words, so that "four8two" is two number words; anything else removed joins its neighbours, so that a zero-width space cannot hide "four" as "f\u{200b}our".
    let separated = |character: char| character.is_numeric() || character.is_whitespace();
    let mut text = String::new();
    for character in name
        .chars()
        .map(|character| if separated(character) { ' ' } else { character })
        .nfkc()
    {
        if separated(character) {
            text.push(' ');
        } else if character.is_alphabetic() || NAME_PUNCTUATION.contains(&character) {
            text.push(character);
        }
    }
    let mut kept = String::with_capacity(text.len());
    let mut word = String::new();
    for character in text.chars().map(Some).chain([None]) {
        match character {
            Some(letter) if letter.is_alphabetic() => word.push(letter),
            other => {
                if !number_word(&word) {
                    kept.push_str(&word);
                }
                word.clear();
                kept.extend(other);
            }
        }
    }
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `word`, with its case and lookalike letters folded, says a number.
fn number_word(word: &str) -> bool {
    use unicode_normalization::UnicodeNormalization;
    if word.is_empty() {
        return false;
    }
    let folded: String = word
        .nfd()
        .filter(|character| !unicode_normalization::char::is_combining_mark(*character))
        .map(folded)
        .collect();
    NUMBER_WORDS.contains(&folded.as_str())
}

/// `character` in lower case, as the Latin letter it looks like, if it is a common lookalike of one, from Cyrillic, Greek, Armenian or Latin's small capitals.
fn folded(character: char) -> char {
    match character {
        'А' | 'а' | 'Α' | 'α' | 'ᴀ' => 'a',
        'В' | 'Β' | 'ʙ' => 'b',
        'С' | 'с' | 'ᴄ' => 'c',
        'ԁ' | 'ᴅ' => 'd',
        'Е' | 'е' | 'Ε' | 'ε' | 'ᴇ' => 'e',
        'ғ' => 'f',
        'ɡ' | 'ɢ' => 'g',
        'Н' | 'Η' | 'һ' | 'հ' | 'ʜ' => 'h',
        'І' | 'і' | 'Ι' | 'ι' | 'ı' | 'ɪ' => 'i',
        'Ј' | 'ј' | 'ȷ' | 'ᴊ' => 'j',
        'К' | 'к' | 'Κ' | 'κ' | 'ᴋ' => 'k',
        'ʟ' => 'l',
        'М' | 'м' | 'Μ' | 'ᴍ' => 'm',
        'Ν' | 'η' | 'п' | 'ո' | 'ɴ' => 'n',
        'О' | 'о' | 'Ο' | 'ο' | 'օ' | 'ᴏ' => 'o',
        'Р' | 'р' | 'Ρ' | 'ρ' | 'ᴘ' => 'p',
        'ԛ' => 'q',
        'г' | 'ʀ' => 'r',
        'Ѕ' | 'ѕ' | 'ꜱ' => 's',
        'Т' | 'т' | 'Τ' | 'τ' | 'ᴛ' => 't',
        'υ' | 'ս' | 'ᴜ' => 'u',
        'ν' | 'ѵ' | 'ᴠ' => 'v',
        'ԝ' | 'ω' | 'ᴡ' => 'w',
        'Х' | 'х' | 'Χ' | 'χ' => 'x',
        'У' | 'у' | 'Υ' | 'ʏ' => 'y',
        'Ζ' | 'ᴢ' => 'z',
        other => other.to_lowercase().next().unwrap_or(other),
    }
}

/// A code as someone typed it, with spaces removed: six digits, or `None`.
pub fn typed(text: &str) -> Option<String> {
    let digits: String = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_round_trip_and_commit() {
        let nonce = Nonce::generate();
        assert_ne!(nonce, Nonce::generate());
        assert_eq!(Nonce::parse(&nonce.to_hex()), Some(nonce));
        assert_eq!(nonce.commitment().len(), 64);
        assert_ne!(nonce.commitment(), nonce.to_hex());
        // A pair whose second digit is not hexadecimal is refused as well as one whose first is.
        for text in [
            "",
            "abc",
            &"g".repeat(64),
            &"0g".repeat(32),
            &"0".repeat(63),
        ] {
            assert_eq!(Nonce::parse(text), None, "{text}");
        }
        assert!(!format!("{nonce:?}").contains(&nonce.to_hex()));
    }

    #[test]
    fn both_ends_derive_the_same_code_only_from_the_same_certificate() {
        let ours = Fingerprint::of(b"ours");
        let device = Nonce::generate();
        let server = Nonce::generate();
        let code = code(ours, device, server);
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
        assert_eq!(super::code(ours, device, server), code);
        // A relay with another certificate cannot steer the codes to match: whatever server value it chooses for the device, having seen only the commitment, the device's code depends on a value it does not know. Here, a thousand choices all differ from the real server's code but by chance.
        let theirs = Fingerprint::of(b"theirs");
        let matches = (0..1000)
            .filter(|_| super::code(theirs, device, Nonce::generate()) == code)
            .count();
        assert!(matches <= 1, "{matches}");
    }

    /// What is shown of `name`, as a device chose it.
    fn shown(name: &str) -> String {
        ChosenName::new(name).to_string()
    }

    #[test]
    fn a_chosen_name_carries_no_numerals_in_any_script() {
        assert_eq!(shown("Quest 3"), "a device calling itself \"Quest\"");
        assert_eq!(
            shown("Type 482913 to confirm"),
            "a device calling itself \"Type to confirm\""
        );
        assert_eq!(shown("Quest 482 913"), "a device calling itself \"Quest\"");
        // Full-width, Arabic-Indic, Devanagari, superscript, circled and Roman numerals, and mathematical digits.
        assert_eq!(
            shown("Code ４８２９１３"),
            "a device calling itself \"Code\""
        );
        assert_eq!(shown("Code ٤٨٢٩١٣"), "a device calling itself \"Code\"");
        assert_eq!(shown("Code ۴۸۲"), "a device calling itself \"Code\"");
        assert_eq!(shown("Code ४८२"), "a device calling itself \"Code\"");
        assert_eq!(shown("Code ⁴⁸² ①②"), "a device calling itself \"Code\"");
        assert_eq!(shown("Code ⅣⅧ"), "a device calling itself \"Code\"");
        assert_eq!(shown("Code 𝟒𝟖𝟐"), "a device calling itself \"Code\"");
        // Full-width letters become plain ones.
        assert_eq!(shown("Ｑｕｅｓｔ"), "a device calling itself \"Quest\"");
        // A numeral separates words.
        assert_eq!(shown("Quest3Pro"), "a device calling itself \"Quest Pro\"");
    }

    #[test]
    fn a_chosen_name_says_no_number_words() {
        assert_eq!(
            shown("Quest — code four eight two nine one three"),
            "a device calling itself \"Quest code\""
        );
        assert_eq!(shown("FOUR Eight tWo"), "a device");
        assert_eq!(
            shown("zero one two three four five six seven eight nine Quest"),
            "a device calling itself \"Quest\""
        );
        assert_eq!(
            shown("oh nil nought naught Ada"),
            "a device calling itself \"Ada\""
        );
        assert_eq!(shown("forty-eight, ninety-one"), "a device");
        assert_eq!(shown("Four8two"), "a device");
        // Full-width letters, lookalikes from other scripts and accents are folded before comparing.
        assert_eq!(shown("ｆｏｕｒ Quest"), "a device calling itself \"Quest\"");
        assert_eq!(
            shown("f\u{43e}ur Quest"),
            "a device calling itself \"Quest\""
        );
        assert_eq!(
            shown("\u{39d}\u{399}\u{39d}\u{395} Quest"),
            "a device calling itself \"Quest\""
        );
        assert_eq!(shown("fóur Quest"), "a device calling itself \"Quest\"");
        // Only whole words: these merely contain one.
        assert_eq!(
            shown("Phone Often"),
            "a device calling itself \"Phone Often\""
        );
        assert_eq!(
            shown("Someone's Tent"),
            "a device calling itself \"Someone's Tent\""
        );
    }

    #[test]
    fn lookalike_letters_fold_to_the_latin_letters_they_look_like() {
        for (lookalikes, letter) in [
            ("АаΑαᴀ", 'a'),
            ("ВΒʙ", 'b'),
            ("Ссᴄ", 'c'),
            ("ԁᴅ", 'd'),
            ("ЕеΕεᴇ", 'e'),
            ("ғ", 'f'),
            ("ɡɢ", 'g'),
            ("НΗһհʜ", 'h'),
            ("ІіΙιıɪ", 'i'),
            ("Јјȷᴊ", 'j'),
            ("КкΚκᴋ", 'k'),
            ("ʟ", 'l'),
            ("МмΜᴍ", 'm'),
            ("Νηпոɴ", 'n'),
            ("ОоΟοօᴏ", 'o'),
            ("РрΡρᴘ", 'p'),
            ("ԛ", 'q'),
            ("гʀ", 'r'),
            ("Ѕѕꜱ", 's'),
            ("ТтΤτᴛ", 't'),
            ("υսᴜ", 'u'),
            ("νѵᴠ", 'v'),
            ("ԝωᴡ", 'w'),
            ("ХхΧχ", 'x'),
            ("УуΥʏ", 'y'),
            ("Ζᴢ", 'z'),
            ("Aa", 'a'),
        ] {
            for lookalike in lookalikes.chars() {
                assert_eq!(folded(lookalike), letter, "{lookalike}");
            }
        }
        // So a number word in lookalikes is one: "seven" with a Cyrillic "е" and Greek "ν", and "six" with a Cyrillic "х".
        assert_eq!(
            shown("s\u{435}\u{3bd}en si\u{445} Quest"),
            "a device calling itself \"Quest\""
        );
    }

    #[test]
    fn a_chosen_name_hides_and_reorders_nothing() {
        // A zero-width space cannot split a number word, nor a bidirectional override reverse one.
        assert_eq!(
            shown("f\u{200b}our Quest"),
            "a device calling itself \"Quest\""
        );
        assert_eq!(
            shown("\u{202e}ruof\u{202c} Quest"),
            "a device calling itself \"ruof Quest\""
        );
        assert_eq!(
            shown("Ada\u{2066}\u{200d}\u{feff}\u{e0041}'s\u{2069} Quest"),
            "a device calling itself \"Ada's Quest\""
        );
        // Control characters and line breaks, and a quotation mark that would end the quoted name early.
        assert_eq!(
            shown("Ada\u{7}\n\r\u{2028}\tQuest"),
            "a device calling itself \"Ada Quest\""
        );
        assert_eq!(
            shown("Quest\" is safe, type \"the code"),
            "a device calling itself \"Quest is safe, type the code\""
        );
        // Symbols, such as emoji and a number sign, are removed; common punctuation stays.
        assert_eq!(
            shown("Ada's #Quest 🎧 (Pro) & co."),
            "a device calling itself \"Ada's Quest (Pro) & co.\""
        );
        assert_eq!(
            shown("   Ada    Quest   "),
            "a device calling itself \"Ada Quest\""
        );
    }

    #[test]
    fn a_long_chosen_name_is_cut() {
        let name = ChosenName::new(&"Quest ".repeat(20));
        let kept = name.as_str().unwrap();
        assert!(kept.ends_with('…'), "{kept}");
        assert_eq!(kept.chars().count(), NAME_CHARACTERS);
        assert_eq!(kept, "Quest Quest Quest Quest Quest Q…");
        // A cut that would leave a number word, such as "seven" of "sevenfold", removes it.
        let cut = ChosenName::new("Abcdefghijklmnopqrstuvwxy sevenfold");
        assert_eq!(cut.as_str(), Some("Abcdefghijklmnopqrstuvwxy…"));
        // A cut leaving no letter is no name.
        let nothing = ChosenName::new(&format!("{}Quest", "- ".repeat(16)));
        assert_eq!(nothing.as_str(), None);
    }

    #[test]
    fn a_chosen_name_with_nothing_left_is_a_device() {
        for name in [
            "",
            "   ",
            "123 456",
            "٤٨٢٩١٣",
            "four eight two",
            "\u{200b}\u{202e}",
            "🎧 #",
            "-- ++ !!",
        ] {
            let chosen = ChosenName::new(name);
            assert_eq!(chosen.as_str(), None, "{name:?}");
            assert_eq!(chosen.to_string(), "a device", "{name:?}");
        }
    }

    #[test]
    fn codes_are_shown_in_two_groups_and_typed_with_or_without_spaces() {
        assert_eq!(display("042917"), "042 917");
        assert_eq!(display("42"), "42");
        assert_eq!(typed(" 042 917 "), Some("042917".to_owned()));
        assert_eq!(typed("042917"), Some("042917".to_owned()));
        assert_eq!(typed("04291"), None);
        assert_eq!(typed("04291x"), None);
        assert_eq!(typed("decline"), None);
    }
}
