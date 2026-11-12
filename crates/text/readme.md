# `tomori-text`

The fonts that [Tomori](https://github.com/base-rs/tomori)'s panels and labels are drawn with, and the signed-distance-field glyph atlas that they are drawn from, free of GPU code. [`tomori-scene`](https://crates.io/crates/tomori-scene) lays text out on its grid of cells with it, and [`tomori-render`](https://crates.io/crates/tomori-render) uploads the atlas into a texture and draws it.

The atlas is filled on demand: the first time a character is drawn, its outline is rasterised and turned into a distance field in a tile of the atlas, which then never moves, so a mesh built from it stays valid. A shader draws the glyph's edge where the field crosses one half, which stays crisp and antialiased at any distance.

## Fonts

The fonts are subsets of Noto fonts, under the SIL Open Font License 1.1, in [`assets/fonts`](assets/fonts/readme.md) with their licences; `scripts/subset-fonts.sh` in Tomori's repository rebuilds them from pinned upstream releases. `fonts::LICENSES` holds the licences, for a program's `--licenses`, and an APK that includes this crate carries them in its assets.

- Noto Sans Mono: Latin, including Vietnamese, Greek, Cyrillic, punctuation, box drawing and most symbols.
- Noto Sans Symbols 2 and Noto Sans Math: the dingbats and arrows the monospaced face lacks, such as ✓ and ↵.
- Noto Sans JP: the kana, punctuation and first and second level kanji of JIS X 0208, 6,355 kanji, for Japanese.

Characters no face covers, such as emoji, Hangul or rarer kanji, are left to layout to replace.

## Usage

```sh
cargo add tomori-text
```

```rust
let glyph = tomori_text::atlas::glyph('語').expect("Noto Sans JP covers it");
assert_eq!(glyph.advance, 1.0);
```

Its interfaces change with the environment framework ([#77](https://github.com/base-rs/tomori/issues/77)).

## Licence

The code is under the [MIT License](license.md), and the bundled fonts under the [SIL Open Font License 1.1](assets/fonts/readme.md), so the package's licence is `MIT AND OFL-1.1`: a program that includes it follows both, which for the fonts means shipping their licences with them, as `fonts::LICENSES` makes easy.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-scene`](https://crates.io/crates/tomori-scene), which lays text out.
- [`tomori-render`](https://crates.io/crates/tomori-render), which draws it.
