// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! KTX2, the Khronos texture container, written as `tomori-render` reads it: one 2D image of block-compressed levels,
//! not supercompressed, with its data format descriptor, its levels stored smallest first, each aligned to its blocks.
use crate::{Chain, Format};

/// KTX2's identifier, its first twelve bytes.
pub const IDENTIFIER: [u8; 12] = [
    0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n',
];

/// The header and its index before the levels' index.
const HEADER: usize = 80;
/// Each level's entry in the index: its offset, its length and its length uncompressed.
const LEVEL: usize = 24;
/// The data format descriptor: its total size, a basic block of 24 bytes and one sample of 16.
const DESCRIPTOR: usize = 44;
/// A block's bytes, and what each level's data is aligned to.
const BLOCK: usize = 16;

/// The Vulkan format of `format`, in sRGB or linear.
pub fn vk_format(format: Format, srgb: bool) -> u32 {
    match (format, srgb) {
        // VK_FORMAT_ASTC_4x4_UNORM_BLOCK and VK_FORMAT_ASTC_4x4_SRGB_BLOCK.
        (Format::Astc, false) => 157,
        (Format::Astc, true) => 158,
        // VK_FORMAT_BC7_UNORM_BLOCK and VK_FORMAT_BC7_SRGB_BLOCK.
        (Format::Bc7, false) => 145,
        (Format::Bc7, true) => 146,
    }
}

/// The data format descriptor of a 4×4 block of 16 bytes of `format`: its colour model, BT.709's primaries, sRGB's or
/// linear transfer, and one sample of all 128 bits.
fn descriptor(format: Format, srgb: bool) -> [u8; DESCRIPTOR] {
    let mut bytes = [0u8; DESCRIPTOR];
    bytes[..4].copy_from_slice(&(DESCRIPTOR as u32).to_le_bytes());
    // Khronos's vendor and the basic descriptor type, both 0; version 2, and the block's size.
    bytes[8..12].copy_from_slice(&(2u32 | (40 << 16)).to_le_bytes());
    // KHR_DF_MODEL_ASTC or KHR_DF_MODEL_BC7, KHR_DF_PRIMARIES_BT709, KHR_DF_TRANSFER_SRGB or LINEAR, straight alpha.
    bytes[12] = match format {
        Format::Astc => 162,
        Format::Bc7 => 136,
    };
    bytes[13] = 1;
    bytes[14] = if srgb { 2 } else { 1 };
    // Blocks of 4 by 4, each dimension less one, and 16 bytes in the first plane.
    bytes[16] = 3;
    bytes[17] = 3;
    bytes[20] = BLOCK as u8;
    // The sample: from bit 0, 128 bits less one, of the format's one channel, which both formats number 0.
    bytes[30] = 127;
    // Its upper bound, all ones.
    bytes[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes
}

/// `chain`, each level compressed to `format`'s blocks by `encode`, as a KTX2 file of the Vulkan format `vk_format`.
pub fn write(
    chain: &Chain,
    format: Format,
    srgb: bool,
    encode: impl Fn(&[[u8; 4]; 16]) -> [u8; 16] + Sync,
) -> Vec<u8> {
    let levels: Vec<Vec<u8>> = (0..chain.levels.len())
        .map(|level| blocks(chain, level, &encode))
        .collect();
    let count = levels.len();
    let descriptor_at = HEADER + count * LEVEL;
    let mut offsets = vec![0usize; count];
    // Smallest first, as KTX2 lays them out so that a reader can stop early.
    let mut end = descriptor_at + DESCRIPTOR;
    for level in (0..count).rev() {
        end = end.next_multiple_of(BLOCK);
        offsets[level] = end;
        end += levels[level].len();
    }
    let mut bytes = Vec::with_capacity(end);
    bytes.extend_from_slice(&IDENTIFIER);
    for word in [
        vk_format(format, srgb),
        // Each block is read as one, as compressed formats are.
        1,
        chain.width,
        chain.height,
        // Neither deep nor layered, and one face.
        0,
        0,
        1,
        count as u32,
        // Not supercompressed.
        0,
        descriptor_at as u32,
        DESCRIPTOR as u32,
        // No key and value data.
        0,
        0,
    ] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    // No supercompression's global data.
    bytes.extend_from_slice(&[0; 16]);
    for (offset, level) in offsets.iter().zip(&levels) {
        for long in [*offset, level.len(), level.len()] {
            bytes.extend_from_slice(&(long as u64).to_le_bytes());
        }
    }
    bytes.extend_from_slice(&descriptor(format, srgb));
    for level in (0..count).rev() {
        bytes.resize(offsets[level], 0);
        bytes.extend_from_slice(&levels[level]);
    }
    bytes
}

/// Level `level` of `chain` as blocks, row by row, each from the texels it covers, those past a level smaller than a
/// block taken from where the tile wraps to; a band of rows on each of the machine's threads, which changes nothing of
/// what each block is.
fn blocks(
    chain: &Chain,
    level: usize,
    encode: &(impl Fn(&[[u8; 4]; 16]) -> [u8; 16] + Sync),
) -> Vec<u8> {
    let (width, height) = chain.size(level);
    let texels = &chain.levels[level];
    let (across, down) = (width.div_ceil(4) as usize, height.div_ceil(4) as usize);
    let mut bytes = vec![0u8; across * down * BLOCK];
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(down);
    let rows = down.div_ceil(threads);
    std::thread::scope(|scope| {
        for (band, chunk) in bytes.chunks_mut(rows * across * BLOCK).enumerate() {
            scope.spawn(move || {
                for (index, out) in chunk.chunks_mut(BLOCK).enumerate() {
                    let (column, row) = (
                        (index % across) as u32,
                        (band * rows + index / across) as u32,
                    );
                    let block: [[u8; 4]; 16] = std::array::from_fn(|texel| {
                        let x = (column * 4 + texel as u32 % 4) % width;
                        let y = (row * 4 + texel as u32 / 4) % height;
                        let at = ((y * width + x) * 4) as usize;
                        [texels[at], texels[at + 1], texels[at + 2], texels[at + 3]]
                    });
                    out.copy_from_slice(&encode(&block));
                }
            });
        }
    });
    bytes
}
