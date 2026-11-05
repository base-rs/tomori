#!/usr/bin/env bash
# Rebuild the fonts `tomori-text` bundles, in crates/text/assets/fonts, from pinned upstream Noto releases: download them into DIRECTORY (default target/downloads/fonts), check their checksums, and keep only the characters Tomori draws with fontTools' `pyftsubset`, which must be installed. See crates/text/assets/fonts/readme.md.
set -euo pipefail
cd "$(dirname "$0")/.."
downloads="${1:-target/downloads/fonts}"
output=crates/text/assets/fonts
command -v pyftsubset > /dev/null || { echo "pyftsubset (fontTools) is needed: install python-fonttools." >&2; exit 1; }
mkdir -p "$downloads" "$output"

noto=https://raw.githubusercontent.com/notofonts/notofonts.github.io/ac9cdcd317b3c34c7578f609189c963c7eeff3c5/fonts
cjk=https://raw.githubusercontent.com/notofonts/noto-cjk/523d033d6cb47f4a80c58a35753646f5c3608a78/Sans/SubsetOTF/JP
fetch() {
    local url="$1" name="$2" sum="$3"
    if [[ ! -s "$downloads/$name" ]]; then curl --fail --location --retry 3 "$url" -o "$downloads/$name"; fi
    printf '%s  %s\n' "$sum" "$downloads/$name" | sha256sum --strict -c -
}
fetch "$noto/NotoSansMono/unhinted/ttf/NotoSansMono-Regular.ttf" NotoSansMono-Regular.ttf 87f8ce0522a6c99b743ee5fc75b4073cfdd575639119672828b7b9944b65b4f4
fetch "$noto/NotoSansSymbols2/unhinted/ttf/NotoSansSymbols2-Regular.ttf" NotoSansSymbols2-Regular.ttf c4a0a80f0041ce4be81e2478faad22776d23edb98ae3f0d19bd37044820ecf9d
fetch "$noto/NotoSansMath/unhinted/ttf/NotoSansMath-Regular.ttf" NotoSansMath-Regular.ttf b127e84699212b6b2ef50aff58e0ebebeec04ffe6db1b9eb9e209c8c3d97b4aa
fetch "$cjk/NotoSansJP-Regular.otf" NotoSansJP-Regular.otf dff723ba59d57d136764a04b9b2d03205544f7cd785a711442d6d2d085ac5073

# Outlines and names only: the atlas draws glyphs one cell at a time, without hinting or OpenType layout.
subset() {
    local font="$1" out="$2" unicodes="$3"
    pyftsubset "$downloads/$font" --output-file="$output/$out" --unicodes="$unicodes" \
        --layout-features='' --no-hinting --name-IDs='*' --name-languages='*' --notdef-outline \
        --drop-tables+=DSIG,GSUB,GPOS,GDEF,BASE,JSTF,MATH,VORG,vhea,vmtx
}
# Latin, including Vietnamese and Latin Extended-B, Greek, Cyrillic, punctuation, currency, letterlike symbols, number forms, arrows, operators, technical symbols, box drawing, blocks and geometric shapes.
subset NotoSansMono-Regular.ttf NotoSansMono-Regular.ttf 'U+0020-024F,U+0250-02FF,U+0300-036F,U+0370-03FF,U+0400-052F,U+1E00-1EFF,U+2000-206F,U+2070-209F,U+20A0-20CF,U+2100-214F,U+2150-218F,U+2190-21FF,U+2200-22FF,U+2300-23FF,U+2500-259F,U+25A0-25FF,U+FFFD'
# What the monospaced face lacks: check marks, crosses, stars and other dingbats, ballot boxes, media controls and technical symbols.
subset NotoSansSymbols2-Regular.ttf NotoSansSymbols2-Regular.ttf 'U+2300-23FF,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+2B00-2BFF'
# Arrows, such as the return arrow that marks where code is broken, and the midline ellipsis.
subset NotoSansMath-Regular.ttf NotoSansMath-Regular.ttf 'U+2190-21FF,U+22EF,U+27F0-27FF,U+2900-297F'
# Japanese: JIS X 0208, every kanji of its first and second levels and its kana, punctuation and symbols, with the kana, CJK punctuation and full-width forms blocks entire. Other CJK characters are left out, to keep the APK small; see the readme.
jis="$(python3 -I -c '
codes = set()
for row in list(range(1, 9)) + list(range(16, 85)):
    for cell in range(1, 95):
        try:
            codes.add(ord(bytes([0xA0 + row, 0xA0 + cell]).decode("euc_jp")))
        except UnicodeDecodeError:
            pass
print(",".join("U+%04X" % code for code in sorted(code for code in codes if code > 0x7F)))
')"
subset NotoSansJP-Regular.otf NotoSansJP-Regular.otf "U+3000-303F,U+3040-309F,U+30A0-30FF,U+31F0-31FF,U+FF00-FFEF,$jis"
ls -l "$output"
