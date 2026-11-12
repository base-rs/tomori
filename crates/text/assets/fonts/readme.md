# Bundled fonts

Subsets of four Noto fonts, cut by `scripts/subset-fonts.sh` with fontTools' `pyftsubset` to the characters Tomori draws, with their outlines and names only: no hinting and no OpenType layout tables, as the atlas draws one glyph per cell. The script downloads the originals, checks them against the checksums below, and with fontTools 4.68 makes these files byte for byte.

| File | From | Upstream SHA-256 | Licence |
| --- | --- | --- | --- |
| `NotoSansMono-Regular.ttf` | Noto Sans Mono 2.014, `fonts/NotoSansMono/unhinted/ttf/NotoSansMono-Regular.ttf` in [notofonts.github.io at `ac9cdcd`](https://github.com/notofonts/notofonts.github.io/tree/ac9cdcd317b3c34c7578f609189c963c7eeff3c5) | `87f8ce0522a6c99b743ee5fc75b4073cfdd575639119672828b7b9944b65b4f4` | [OFL 1.1](NotoSansMono-OFL.txt), from [latin-greek-cyrillic](https://github.com/notofonts/latin-greek-cyrillic/blob/0ef94805d19e98ed658543a7c48c0a4f19cc0ac0/OFL.txt) |
| `NotoSansSymbols2-Regular.ttf` | Noto Sans Symbols 2 2.008, `fonts/NotoSansSymbols2/unhinted/ttf/NotoSansSymbols2-Regular.ttf`, the same commit | `c4a0a80f0041ce4be81e2478faad22776d23edb98ae3f0d19bd37044820ecf9d` | [OFL 1.1](NotoSansSymbols2-OFL.txt), from [symbols](https://github.com/notofonts/symbols/blob/c42cbaee99cb13013814864f8081d8c18656ec02/OFL.txt) |
| `NotoSansMath-Regular.ttf` | Noto Sans Math 3.000, `fonts/NotoSansMath/unhinted/ttf/NotoSansMath-Regular.ttf`, the same commit | `b127e84699212b6b2ef50aff58e0ebebeec04ffe6db1b9eb9e209c8c3d97b4aa` | [OFL 1.1](NotoSansMath-OFL.txt), from [math](https://github.com/notofonts/math/blob/50579ac1e067bde237034eb1220b5e89bd62dde3/OFL.txt) |
| `NotoSansJP-Regular.otf` | Noto Sans CJK 2.004's Japanese subset, `Sans/SubsetOTF/JP/NotoSansJP-Regular.otf` in [noto-cjk at `Sans2.004`](https://github.com/notofonts/noto-cjk/tree/523d033d6cb47f4a80c58a35753646f5c3608a78) | `dff723ba59d57d136764a04b9b2d03205544f7cd785a711442d6d2d085ac5073` | [OFL 1.1](NotoSansJP-LICENSE.txt), © 2014-2021 Adobe |

None of the licences reserves a font name, so the subsets keep theirs.

What each keeps:

- **Noto Sans Mono** (162 KB, from 389 KB): Latin, Latin-1, Latin Extended-A, -B and Additional, IPA, combining marks, Greek, Cyrillic, general punctuation, super- and subscripts, currency, letterlike symbols, number forms, arrows, mathematical operators, technical symbols, box drawing, block elements and geometric shapes.
- **Noto Sans Symbols 2** (91 KB, from 672 KB): technical symbols, geometric shapes, miscellaneous symbols, dingbats and miscellaneous symbols and arrows, such as ✓, ✕, ⚠ and ★.
- **Noto Sans Math** (25 KB, from 642 KB): arrows, supplemental arrows and the midline ellipsis, such as ↵, which marks where code is broken.
- **Noto Sans JP** (1.46 MB, from 4.53 MB): every character of JIS X 0208 above ASCII, which is its kana, punctuation, symbols and 6,355 kanji of the first and second levels, with the CJK punctuation, hiragana, katakana and full-width forms blocks entire.

## CJK beyond JIS X 0208

JIS X 0208's kanji cover nearly all everyday Japanese. Chinese and Korean, and Japanese kanji outside it, are not bundled: the full Noto Sans CJK is 16 MB a weight, against about 1.5 MB for this subset. Such characters are drawn as `?`. Language packs ([#103](https://github.com/base-rs/tomori/issues/103)) can load more on demand: a face for the space's language fetched from the server and cached by its hash, or, on Android, the system's own `/system/fonts/NotoSansCJK-Regular.ttc` where it has one, appended to the faces after these.
