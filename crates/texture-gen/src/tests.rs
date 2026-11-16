// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// FNV-1a of `bytes`, to pin what a recipe makes without a hashing crate.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The mean absolute difference, per channel, between the texels of column or row `a` and `b` of level 0.
fn difference(chain: &Chain, pairs: impl Iterator<Item = (usize, usize)>) -> f32 {
    let texels = &chain.levels[0];
    let mut sum = 0u64;
    let mut count = 0u64;
    for (a, b) in pairs {
        for channel in 0..3 {
            sum += u64::from(texels[a * 4 + channel].abs_diff(texels[b * 4 + channel]));
            count += 1;
        }
    }
    sum as f32 / count as f32
}

#[test]
fn a_recipe_makes_the_same_bytes_every_time_and_another_seed_others() {
    for surface in Surface::ALL {
        let recipe = Recipe::new(surface, 64, 11);
        let (once, again) = (recipe.generate(), recipe.generate());
        assert_eq!(once, again);
        for format in Format::ALL {
            assert_eq!(
                once.color.ktx2(format, true),
                again.color.ktx2(format, true)
            );
        }
        assert_ne!(Recipe::new(surface, 64, 12).generate().color, once.color);
    }
    // Pinned, so that a change to what a recipe makes, here or on another machine, is seen: the farmyard names its own
    // textures by digest, and its tests pin those.
    let pinned: Vec<u64> = Surface::ALL
        .iter()
        .map(|surface| {
            let set = Recipe::new(*surface, 32, 5).generate();
            fnv(&set.color.ktx2(Format::Astc, true))
                ^ fnv(&set.normal.ktx2(Format::Bc7, false)).rotate_left(1)
        })
        .collect();
    assert_eq!(
        pinned,
        [
            0xe61e_5ae0_6040_b637,
            0x6fe8_2c82_86a4_aed3,
            0xdd77_4ffb_30cd_750b
        ],
        "{pinned:x?}"
    );
}

#[test]
fn a_tile_shows_no_edge_where_it_meets_itself() {
    for surface in Surface::ALL {
        let size = 256usize;
        let set = Recipe::new(surface, size as u32, 3).generate();
        for chain in [&set.color, &set.normal] {
            let at = |x: usize, y: usize| y * size + x;
            // How much each column differs from the next, the last from the first where the tile meets itself, and each
            // row from the next likewise.
            let columns: Vec<f32> = (0..size)
                .map(|x| difference(chain, (0..size).map(|y| (at(x, y), at((x + 1) % size, y)))))
                .collect();
            let rows: Vec<f32> = (0..size)
                .map(|y| difference(chain, (0..size).map(|x| (at(x, y), at(x, (y + 1) % size)))))
                .collect();
            for (lines, name) in [(columns, "across"), (rows, "down")] {
                // The edge is a line like the others: no further from its neighbour than the furthest of the rest.
                let edge = lines[size - 1];
                let furthest = lines[..size - 1].iter().copied().fold(0.0, f32::max);
                assert!(
                    edge <= furthest,
                    "{surface:?} {name}: {edge}, at most {furthest}"
                );
            }
            // Lines half a tile apart differ as neighbours do not, across or, for bark, whose fibres run down, down: the
            // test would see an edge.
            let far = difference(chain, (0..size).map(|y| (at(size / 2, y), at(0, y))))
                / difference(chain, (0..size).map(|y| (at(0, y), at(1, y))));
            let far_down = difference(chain, (0..size).map(|x| (at(x, size / 2), at(x, 0))))
                / difference(chain, (0..size).map(|x| (at(x, 0), at(x, 1))));
            assert!(far.max(far_down) > 1.5, "{surface:?} {far} {far_down}");
        }
    }
}

#[test]
fn every_level_is_made_down_to_one_texel_in_linear_light() {
    let set = Recipe::new(Surface::Stone, 64, 1).generate();
    for chain in [&set.color, &set.normal] {
        assert_eq!(chain.levels.len(), 7);
        for (level, bytes) in chain.levels.iter().enumerate() {
            let (width, height) = chain.size(level);
            assert_eq!(
                (width, height),
                ((64 >> level).max(1), (64 >> level).max(1))
            );
            assert_eq!(bytes.len(), (width * height * 4) as usize);
            assert!(bytes.chunks(4).all(|texel| texel[3] == 255));
        }
    }
    // The last level is the mean of the first, in linear light, as the flat colour is.
    let last = &set.color.levels[6];
    for channel in 0..3 {
        assert!(
            (srgb::decode(last[channel]) - set.flat[channel]).abs() < 0.003,
            "{channel}"
        );
    }
    // Normals are of unit length, facing out, at every level.
    for bytes in &set.normal.levels {
        for texel in bytes.chunks(4) {
            let normal = [0, 1, 2].map(|channel| f32::from(texel[channel]) / 127.5 - 1.0);
            let length = normal.iter().map(|value| value * value).sum::<f32>().sqrt();
            assert!((length - 1.0).abs() < 0.02 && normal[2] > 0.0, "{normal:?}");
        }
    }
    // A flat surface's normals face straight out.
    assert_eq!(normalize([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
}

#[test]
fn a_ktx2_file_is_read_by_another_reader_and_its_blocks_by_other_decoders() {
    let set = Recipe::new(Surface::Earth, 32, 9).generate();
    for format in Format::ALL {
        for (chain, srgb) in [(&set.color, true), (&set.normal, false)] {
            let bytes = chain.ktx2(format, srgb);
            let reader = ktx2::Reader::new(&bytes[..]).expect("a KTX2 file");
            let header = reader.header();
            assert_eq!(
                header.format.map(|format| format.value()),
                Some(crate::container::vk_format(format, srgb))
            );
            assert_eq!(
                (header.pixel_width, header.pixel_height, header.level_count),
                (32, 32, 6)
            );
            assert_eq!(header.supercompression_scheme, None);
            let descriptor = reader.basic_dfd().expect("a data format descriptor");
            assert_eq!(
                descriptor.color_model.map(|model| model.value()),
                Some(match format {
                    Format::Astc => 162,
                    Format::Bc7 => 136,
                })
            );
            assert_eq!(
                descriptor.texel_block_dimensions.map(|side| side.get()),
                [4, 4, 1, 1]
            );
            for (index, level) in reader.levels().enumerate() {
                let (width, height) = chain.size(index);
                let blocks = (width.div_ceil(4) * height.div_ceil(4)) as usize;
                assert_eq!(level.data.len(), blocks * 16);
                // Levels are aligned to blocks.
                let offset = level.data.as_ptr() as usize - bytes.as_ptr() as usize;
                assert_eq!(offset % 16, 0);
                let source = &chain.levels[index];
                let mut error = 0u64;
                for block in 0..blocks {
                    let data: [u8; 16] =
                        level.data[block * 16..block * 16 + 16].try_into().unwrap();
                    let mut decoded = [[0u8; 4]; 16];
                    match format {
                        Format::Astc => {
                            assert!(astc_decode::astc_decode_block(
                                &data,
                                astc_decode::Footprint::new(4, 4),
                                |x, y, color| {
                                    decoded[(y * 4 + x) as usize] = color;
                                }
                            ));
                        }
                        Format::Bc7 => {
                            let mut texels = [0u8; 64];
                            bcdec_rs::bc7(&data, &mut texels, 16);
                            for (texel, decoded) in texels.chunks(4).zip(&mut decoded) {
                                decoded.copy_from_slice(texel);
                            }
                        }
                    }
                    let across = width.div_ceil(4);
                    let (bx, by) = (block as u32 % across, block as u32 / across);
                    for (texel, got) in decoded.iter().enumerate() {
                        let (x, y) = (bx * 4 + texel as u32 % 4, by * 4 + texel as u32 / 4);
                        if x < width && y < height {
                            let at = ((y * width + x) * 4) as usize;
                            for channel in 0..3 {
                                error += u64::from(got[channel].abs_diff(source[at + channel]));
                            }
                        }
                    }
                }
                let mean = error as f32 / (width * height * 3) as f32;
                assert!(mean < 3.0, "{format:?} {srgb} level {index}: {mean}");
            }
        }
    }
    assert_eq!(Format::ALL.map(Format::name), ["astc", "bc7"]);
}

#[test]
#[should_panic(expected = "a power of two")]
fn a_tile_not_a_power_of_two_is_refused() {
    Recipe::new(Surface::Bark, 48, 0);
}
