// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The bundled faces, in the order a character is looked for in them: Noto Sans Mono for Latin, Greek, Cyrillic, punctuation, box drawing and most symbols; Noto Sans Symbols 2 and Noto Sans Math for the dingbats and arrows it lacks; and Noto Sans JP, cut to the kana, punctuation and first and second level kanji of JIS X 0208, for Japanese. They are compiled in, and their outlines are read only when a glyph is first drawn.
use ab_glyph::{Font, FontRef, GlyphId};
use std::sync::LazyLock;

/// How a face's glyphs sit in their cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One cell per character, centred.
    Mono,
    /// Symbols, scaled down where they are wider than their cell.
    Symbols,
    /// CJK, two cells per character, centred on the cells' height.
    Cjk,
}

/// A bundled face.
pub struct Face {
    pub name: &'static str,
    pub kind: Kind,
    /// Atlas pixels per em that its glyphs are rasterised at: CJK is finer, for its strokes.
    pub scale: f32,
    pub(crate) font: FontRef<'static>,
}

impl Face {
    fn new(name: &'static str, kind: Kind, scale: f32, data: &'static [u8]) -> Self {
        Self {
            name,
            kind,
            scale,
            font: FontRef::try_from_slice(data).expect("a bundled font parses"),
        }
    }

    /// Font units per em.
    pub(crate) fn em(&self) -> f32 {
        self.font.units_per_em().unwrap_or(1000.0)
    }

    fn glyph(&self, ch: char) -> Option<GlyphId> {
        let id = self.font.glyph_id(ch);
        (id.0 != 0).then_some(id)
    }
}

static FACES: LazyLock<[Face; 4]> = LazyLock::new(|| {
    [
        Face::new(
            "Noto Sans Mono",
            Kind::Mono,
            32.0,
            include_bytes!("../assets/fonts/NotoSansMono-Regular.ttf"),
        ),
        Face::new(
            "Noto Sans Symbols 2",
            Kind::Symbols,
            32.0,
            include_bytes!("../assets/fonts/NotoSansSymbols2-Regular.ttf"),
        ),
        Face::new(
            "Noto Sans Math",
            Kind::Symbols,
            32.0,
            include_bytes!("../assets/fonts/NotoSansMath-Regular.ttf"),
        ),
        Face::new(
            "Noto Sans JP",
            Kind::Cjk,
            40.0,
            include_bytes!("../assets/fonts/NotoSansJP-Regular.otf"),
        ),
    ]
});

/// The bundled faces, in the order characters are looked for in them.
pub fn faces() -> &'static [Face] {
    FACES.as_slice()
}

/// The first face with a glyph for `ch`, and the glyph.
pub(crate) fn find(ch: char) -> Option<(&'static Face, GlyphId)> {
    faces()
        .iter()
        .find_map(|face| face.glyph(ch).map(|id| (face, id)))
}

/// Whether a bundled face has a glyph for `ch`.
pub fn covers(ch: char) -> bool {
    find(ch).is_some()
}

/// The bundled fonts' licences, as `--licenses` shows them.
pub const LICENSES: &str = concat!(
    "Noto Sans Mono (SIL Open Font License 1.1):\n\n",
    include_str!("../assets/fonts/NotoSansMono-OFL.txt"),
    "\nNoto Sans Symbols 2 (SIL Open Font License 1.1):\n\n",
    include_str!("../assets/fonts/NotoSansSymbols2-OFL.txt"),
    "\nNoto Sans Math (SIL Open Font License 1.1):\n\n",
    include_str!("../assets/fonts/NotoSansMath-OFL.txt"),
    "\nNoto Sans JP, © 2014-2021 Adobe (SIL Open Font License 1.1):\n\n",
    include_str!("../assets/fonts/NotoSansJP-LICENSE.txt"),
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_parses_and_is_found_in_order() {
        assert_eq!(faces().len(), 4);
        for face in faces() {
            assert_eq!(face.em(), 1000.0, "{}", face.name);
        }
        assert_eq!(find('a').unwrap().0.name, "Noto Sans Mono");
        assert_eq!(find('✓').unwrap().0.name, "Noto Sans Symbols 2");
        assert_eq!(find('↵').unwrap().0.name, "Noto Sans Math");
        assert_eq!(find('日').unwrap().0.kind, Kind::Cjk);
    }

    #[test]
    fn the_faces_cover_what_tomori_draws() {
        // Latin with accents, Vietnamese, Greek, Cyrillic, typographic punctuation, the interface's symbols, box drawing, and Japanese.
        let text = "José Łódź Dvořák Œuvre Nguyễn Thị Hương λπ Жщ “quoted” ‘single’ – — … ⋯ ° × ± ½ € • ● ○ ◐ ✓ ✕ ↵ → ← ↑ ↓ ▾ ▸ │ ─ █ ⚠ ★ こんにちは カタカナ 日本語 漢字 龍 葛 。、「」！ＡＢ";
        let missing: String = text
            .chars()
            .filter(|ch| *ch != ' ' && !covers(*ch))
            .collect();
        assert_eq!(missing, "");
        // Emoji, Hangul and kanji outside JIS X 0208 are not bundled.
        for ch in ['🎤', '한', '𠮷'] {
            assert!(!covers(ch), "{ch}");
        }
    }

    #[test]
    fn the_licences_name_every_face() {
        for face in faces() {
            assert!(LICENSES.contains(face.name), "{}", face.name);
        }
        assert!(LICENSES.contains("SIL OPEN FONT LICENSE Version 1.1"));
    }
}
