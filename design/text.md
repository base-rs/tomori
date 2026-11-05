# Text

This describes how the headset, the desktop window and the spectator draw text: the fonts, the glyph atlas they are drawn from, and the one rule for laying text out in cells.

## Requirement

**Fonts.** Text is drawn from fonts bundled in `tomori-text`, under licences that allow bundling and subsetting, the SIL Open Font License: Noto Sans Mono for Latin, including Vietnamese, Greek, Cyrillic, punctuation, box drawing and most symbols; Noto Sans Symbols 2 and Noto Sans Math for the dingbats and arrows it lacks, such as ✓ and ↵; and Noto Sans JP for Japanese, cut to the kana, punctuation and first and second level kanji of JIS X 0208. A character is drawn from the first of these that has it. Each font is a subset that `scripts/subset-fonts.sh` rebuilds from a pinned, checksummed upstream file, and its licence is shipped with it, shown by `tomori --licenses` and carried in the APK's assets. Together they are 1.74 MB, and the APK, which stores the native library uncompressed, grew by 2.1 MB with them and the code that reads them.

**The glyph atlas.** Glyphs are drawn from one single-channel atlas of signed distance fields, a tile per glyph, shared by every panel, label and sign: a glyph is drawn into it the first time it is shown, and never moves or leaves it, so that a mesh built from it stays valid. The atlas is 2,048 pixels wide and grows downwards, doubling, to at most 8,192 rows, 16 MiB, the largest texture wgpu's default limits allow, holding about 4,000 kanji or many more Latin glyphs; once full, a new character is drawn as `?`. A glyph is one quad. The shader draws its edge where the field crosses one half, antialiased by how fast the field changes across the pixel, so that text stays crisp at any distance and tilt; bold moves the edge outwards. Backgrounds, bars and other filled quads sample the atlas's solid block, so that one pipeline draws both. The GPU texture uploads only the rows the atlas has gained since the last frame.

**Cells.** Text is laid out on a grid of monospaced cells, 9 × 12 font pixels, as before. Each grapheme takes a cell, drawn as the one character its letter and marks compose to, or two for a wide character, such as a CJK ideograph, kana or a full-width form, as East Asian Width says; a glyph is centred in its cells, and box drawing and blocks fill theirs, so that lines join. Control characters, variation selectors and zero-width spaces and joiners take no cell; a mark that composes with nothing is left out. A character no bundled font has is drawn as its base letter where a font has that, and otherwise as `?`, as for emoji, Hangul and kanji outside JIS X 0208. The console prints the same characters.

**Breaking lines.** `ui_text`'s one rule wraps every panel, label and menu row: at spaces, never at a non-breaking one; between CJK characters, which have no spaces, wherever Unicode's line breaking rules (UAX #14) allow, so never before closing punctuation such as "。" or after opening punctuation such as "「"; and, for a word longer than a row, at the row's end. A wide character is never split between rows, or by a cut and its "…". Elsewhere, such as after a hyphen, a Latin word is not broken.

**Not supported.** Text is not shaped: scripts that need it, such as Arabic, Devanagari or Thai, are drawn a character at a time, and right-to-left text is not reordered. Kerning, ligatures, proportional text and vertical Japanese are out of scope, as is colour emoji.

## Rationale

The headset's text was `font8x8` bitmaps drawn as one quad per run of lit pixels: legible for short labels, but it shimmered as the head moved, could not show Japanese (#103), and cost 50 to 80 vertices a glyph, too many for code panels (#151). #146 asked for antialiased, Unicode-capable text from a glyph atlas. A distance field stays sharp when magnified and antialiases itself when minified, from one tile per glyph at one size, which suits text seen from varying distances in VR; one channel, rather than a multi-channel field, keeps CJK tiles small, and its rounder corners do not show at the sizes drawn. The bitmap path was removed rather than kept as a fallback: the fonts are compiled in and parsed by tests, so loading them cannot fail at run time, and an atlas that is full still draws `?`.

The cell grid is kept, so that no layout, row budget or hand menu width changes: the monospaced face fits it, and CJK takes two cells as in a terminal. Noto Sans CJK is 16 MB a weight; JIS X 0208's kanji cover nearly all everyday Japanese in about 1.5 MB. Other CJK can be loaded on demand later, with language packs (#103): a face for the space's language from the server, cached by its hash, or the system's Noto Sans CJK where Android has one. Proposed in #146, and asked for by the maintainer in November 2026.

## Scope

`tomori-text` (fonts, coverage, the atlas), `ui_text` and `canvas` in `tomori-scene` (cells, wrapping and glyph placement), and the panel, label and sign pipelines and `glyphs` in `tomori-render`. Result cards keep their own ASCII layout, which escapes other characters as code points.

## Validation

- Test that the bundled fonts parse, cover the characters Tomori draws, including accented Latin, Vietnamese, Greek, Cyrillic, the interface's symbols and Japanese, and that their licences name every face.
- Test the atlas's packing: tiles never overlap, it grows to its limit and then draws `?`, a glyph is drawn once, and a CJK glyph is finer and an em wide; and the distance field's values against shapes with known distances.
- Test the layout: graphemes are one cell, CJK takes two, lines break between CJK characters but not before closing or after opening punctuation, a wide character is never split, and every hand menu row fits its width in cells, with Japanese titles, names and descriptions.
- Render Latin with accents, Japanese and code with symbols on lavapipe, and check that every glyph draws within a fifth of the ink its tile holds and none is the replacement, that edges are antialiased, that Latin letters share a baseline, that Japanese is centred in its two cells, that code's symbols stay within their cells, and that bold draws more ink.
- On the headset, confirm that the hand menu, the wrist transcript, the recording card, the status pill, workers' labels and signs are legible and steady as the head moves, and log the atlas's size and the time spent drawing glyphs, which the headset's log reports when the atlas grows.
