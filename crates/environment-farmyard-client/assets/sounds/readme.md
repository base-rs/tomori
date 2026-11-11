# The Farmyard's Sounds

The farmyard's ambience plays the sounds in this folder, each named for its slot, `SLOT.ogg`. A slot without a sound is silent, and says so once in the log as the farmyard opens; a sound dropped in here is found by the next build, by the digest of its bytes, embedded everywhere but on Android, whose APK carries this folder as `assets/farmyard/sounds/`. Record each sound's author, source, licence and changes in [`LICENSE.md`](LICENSE.md), and only add sounds whose licence lets Tomori share them: CC0, or CC BY with the credit there. A sound under a licence other than the crate's MIT, such as CC BY, adds it to the crate's `license` in `Cargo.toml`, as `tomori-environment-farmyard`'s chicken does.

## Format

- **Ogg Vorbis**, at 48 kHz (44.1 kHz is resampled as it plays), at about quality 3 to 4.
- **Stereo** for the beds, `wind` and `bush`, and **mono** for every other slot, which is placed in the scene.
- **Loudness**: each normalised to −23 LUFS integrated, its true peak no higher than −3 dBTP, so that the declaration's gains, the participant's level and ducking set the mix, not the files.
- **Loops** (`wind`, `bush`, `river`, `waterfall`, `fire`) loop without a click: cut at zero crossings, or made with a crossfade, with no fade in or out. Calls start and end in silence, with no more than 50 ms of it either side.
- **Size**: at most 1 MiB a file, and 6 MiB together; the build fails beyond either.

## Slots

| Slot | Kind | What it should sound like | Length |
| --- | --- | --- | --- |
| `wind` | bed, stereo, −8 dB | A gentle evening breeze through grass and pines, steady, with no gusts that stand out | 45 to 90 s |
| `bush` | bed, stereo, −10 dB | Distant native bush at dusk: insects, leaves and a far stream, with no birdsong that stands out | 45 to 90 s |
| `river` | loop, mono, +5 dB, heard within 35 m along the river | A shallow river over shingle and stones, close up, steady | 30 to 60 s |
| `waterfall` | loop, mono, +8 dB, heard within 60 m, from the middle of its fall | A small waterfall falling a few metres into a pool | 30 to 60 s |
| `fire` | loop, mono, −2 dB, heard within 12 m, from among the campsite's flames | A campfire crackling close by, steady, with pops but no roar | 20 to 45 s |
| `tui-1` to `tui-3` | call, mono, +14 to +18 dB, a phrase, two or three 1 to 3 s apart, every 25 to 70 s | A tūī's song, its bell notes, clicks and wheezes, one phrase each | 2 to 6 s |
| `bellbird-1` to `bellbird-3` | call, mono, +13 to +17 dB, every 30 to 90 s | A bellbird's (korimako's) liquid, bell-like song, one phrase each | 1 to 4 s |
| `kereru-1`, `kereru-2` | call, mono, +12 to +17 dB, every 90 to 240 s | A kererū's heavy, whooshing wingbeats as it flies over | 1 to 3 s |
| `ruru-1`, `ruru-2` | call, mono, +13 to +17 dB, usually three to six in a row 1.5 to 2.5 s apart, every 120 to 300 s | A ruru's (morepork's) two-note "more-pork" call, once, which runs repeat | 1 to 3 s |

The calls come from the backdrop's bush, about 6 m up in its canopy, at a slightly different pitch and gain each time, a different variant from the last, so that none sounds like a loop; the ruru and the tūī call in runs, each run's interval counting from its end. The water and the birds are raised above the beds because distance takes 17 to 21 dB from them where participants stand; `design/farmyard.md` describes where each is heard, and how loud.
