// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Textures decoded from KTX2, ASTC or RGBA, and PNG, with the levels they carry, refused where they are not what they
//! say or the device cannot sample them, and loaded by digest from the cache onto the GPU.
use super::*;
use crate::test_gpu;

/// A KTX2 texture, each field of its header as written, and its levels' bytes, the largest first, each where its index
/// says, unless `index` says otherwise.
#[derive(Clone)]
struct Ktx2 {
    vk_format: u32,
    width: u32,
    height: u32,
    depth: u32,
    layers: u32,
    faces: u32,
    count: u32,
    supercompression: u32,
    levels: Vec<Vec<u8>>,
    /// Each level's offset and length, if not where and as long as it is.
    index: Option<Vec<(u64, u64)>>,
}

impl Ktx2 {
    /// One image of `vk_format`, `width` by `height`, with `levels`.
    fn new(vk_format: u32, width: u32, height: u32, levels: Vec<Vec<u8>>) -> Self {
        Self {
            vk_format,
            width,
            height,
            depth: 0,
            layers: 0,
            faces: 1,
            count: levels.len() as u32,
            supercompression: 0,
            levels,
            index: None,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = KTX2.to_vec();
        for word in [
            self.vk_format,
            1,
            self.width,
            self.height,
            self.depth,
            self.layers,
            self.faces,
            self.count,
            self.supercompression,
            // The data format descriptor's and key and value data's offsets and lengths, none.
            0,
            0,
            0,
            0,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        // The supercompression global data's offset and length, none.
        bytes.extend([0; 16]);
        let mut offset = (80 + self.levels.len() * 24) as u64;
        let index = self.index.clone().unwrap_or_else(|| {
            self.levels
                .iter()
                .map(|level| {
                    let entry = (offset, level.len() as u64);
                    offset += level.len() as u64;
                    entry
                })
                .collect()
        });
        for (offset, length) in index {
            bytes.extend(offset.to_le_bytes());
            bytes.extend(length.to_le_bytes());
            bytes.extend(length.to_le_bytes());
        }
        for level in &self.levels {
            bytes.extend(level);
        }
        bytes
    }
}

/// An RGBA texture's levels, from `width` by `height` halving down to one texel, each texel its level's number.
fn rgba(width: u32, height: u32) -> Vec<Vec<u8>> {
    let mut levels = Vec::new();
    let (mut width, mut height) = (width, height);
    loop {
        levels.push(vec![levels.len() as u8; (width * height * 4) as usize]);
        if width == 1 && height == 1 {
            return levels;
        }
        (width, height) = ((width / 2).max(1), (height / 2).max(1));
    }
}

/// A PNG of `width` by `height` texels, each a colour of its own.
fn png(width: u32, height: u32) -> (Vec<u8>, Vec<u8>) {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([
            (x as u8).wrapping_mul(40),
            (y as u8).wrapping_mul(60),
            200,
            255 - x as u8,
        ])
    });
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    (bytes, image.into_raw())
}

const ASTC: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_ASTC;

#[test]
fn a_png_is_decoded_to_rgba_in_srgb_as_one_level() {
    let (bytes, texels) = png(3, 2);
    assert_eq!(
        decode(&bytes, wgpu::Features::empty()),
        Ok(Decoded {
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            width: 3,
            height: 2,
            levels: vec![texels],
        })
    );
    // One wider than a texture may be, and one that only begins as a PNG.
    assert_eq!(
        decode(&png(LARGEST + 1, 1).0, wgpu::Features::empty()),
        Err(Error::TooLarge)
    );
    let mut broken = PNG.to_vec();
    broken.extend(b"not a PNG");
    assert!(matches!(
        decode(&broken, wgpu::Features::empty()),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn a_ktx2_texture_of_rgba_or_astc_is_decoded_with_the_levels_it_carries() {
    // RGBA, linear and sRGB, with every level down to one texel.
    for (vk_format, format) in [
        (37, wgpu::TextureFormat::Rgba8Unorm),
        (43, wgpu::TextureFormat::Rgba8UnormSrgb),
    ] {
        let texture = Ktx2::new(vk_format, 4, 2, rgba(4, 2));
        let decoded = decode(&texture.bytes(), wgpu::Features::empty()).unwrap();
        assert_eq!(
            (decoded.format, decoded.width, decoded.height),
            (format, 4, 2)
        );
        assert_eq!(decoded.levels, rgba(4, 2));
    }
    // Asking for mipmaps to be made, by no levels, its one is drawn.
    let mut made = Ktx2::new(37, 4, 2, vec![rgba(4, 2).remove(0)]);
    made.count = 0;
    assert_eq!(
        decode(&made.bytes(), wgpu::Features::empty())
            .unwrap()
            .levels
            .len(),
        1
    );
    // ASTC's 4×4 blocks in sRGB, 8×8 texels and its levels, each block 16 bytes however small the level, kept
    // compressed for a device that samples them.
    let blocks = vec![vec![1; 64], vec![2; 16], vec![3; 16], vec![4; 16]];
    let astc = Ktx2::new(158, 8, 8, blocks.clone()).bytes();
    let decoded = decode(&astc, ASTC).unwrap();
    assert_eq!(
        decoded.format,
        wgpu::TextureFormat::Astc {
            block: wgpu::AstcBlock::B4x4,
            channel: wgpu::AstcChannel::UnormSrgb,
        }
    );
    assert_eq!(decoded.levels, blocks);
    // A device that cannot sample them does not load them.
    assert!(matches!(
        decode(&astc, wgpu::Features::empty()),
        Err(Error::Unsupported(why)) if why.contains("cannot sample")
    ));
    // Each of ASTC's Vulkan formats, by its block and channel.
    assert_eq!(
        format(157),
        Some(wgpu::TextureFormat::Astc {
            block: wgpu::AstcBlock::B4x4,
            channel: wgpu::AstcChannel::Unorm,
        })
    );
    assert_eq!(
        format(166),
        Some(wgpu::TextureFormat::Astc {
            block: wgpu::AstcBlock::B6x6,
            channel: wgpu::AstcChannel::UnormSrgb,
        })
    );
    assert_eq!(
        format(184),
        Some(wgpu::TextureFormat::Astc {
            block: wgpu::AstcBlock::B12x12,
            channel: wgpu::AstcChannel::UnormSrgb,
        })
    );
    assert_eq!(format(185), None);
}

#[test]
fn a_level_is_laid_out_in_whole_blocks() {
    assert_eq!(layout(wgpu::TextureFormat::Rgba8UnormSrgb, 5, 3), (20, 3));
    let six = wgpu::TextureFormat::Astc {
        block: wgpu::AstcBlock::B6x6,
        channel: wgpu::AstcChannel::Unorm,
    };
    assert_eq!(layout(six, 13, 7), (3 * 16, 2));
    assert_eq!(layout(six, 1, 1), (16, 1));
}

#[test]
fn what_is_not_one_whole_texture_it_can_load_is_refused() {
    let none = wgpu::Features::empty();
    let refused = |texture: Ktx2| decode(&texture.bytes(), ASTC).unwrap_err();
    let unsupported = |error: Error| matches!(error, Error::Unsupported(_));
    let invalid = |error: Error| matches!(error, Error::Invalid(_));
    let good = Ktx2::new(37, 4, 4, rgba(4, 4));
    // Neither KTX2 nor PNG, or more than may be read.
    assert!(unsupported(decode(b"GIF89a", none).unwrap_err()));
    assert_eq!(decode(&vec![0; BYTES + 1], none), Err(Error::TooLarge));
    // A header cut short, and an index cut short.
    assert!(invalid(decode(&good.bytes()[..30], none).unwrap_err()));
    assert!(invalid(decode(&good.bytes()[..100], none).unwrap_err()));
    // A format it does not load; more than one 2D image; supercompressed.
    assert!(unsupported(refused(Ktx2 {
        vk_format: 100,
        ..good.clone()
    })));
    for texture in [
        Ktx2 {
            depth: 2,
            ..good.clone()
        },
        Ktx2 {
            layers: 3,
            ..good.clone()
        },
        Ktx2 {
            faces: 6,
            ..good.clone()
        },
        Ktx2 {
            supercompression: 2,
            ..good.clone()
        },
    ] {
        assert!(unsupported(refused(texture)));
    }
    // No texels; too large; not whole blocks.
    assert!(invalid(refused(Ktx2 {
        width: 0,
        ..good.clone()
    })));
    assert_eq!(
        refused(Ktx2 {
            height: LARGEST * 2,
            ..good.clone()
        }),
        Error::TooLarge
    );
    assert!(unsupported(refused(Ktx2::new(
        165,
        8,
        6,
        vec![vec![0; 32]]
    ))));
    // More levels than its size halves to; a level of the wrong length; a level outside it.
    assert!(invalid(refused(Ktx2 {
        count: 4,
        ..good.clone()
    })));
    let mut short = good.clone();
    short.levels[1].pop();
    assert!(invalid(refused(short)));
    let mut outside = good.clone();
    outside.index = Some(vec![(10_000, 64), (10_064, 16), (10_080, 4)]);
    assert!(invalid(refused(outside)));
    let mut wrapping = good.clone();
    wrapping.index = Some(vec![(u64::MAX - 8, 64), (0, 16), (0, 4)]);
    assert!(invalid(refused(wrapping)));
    // Each says why.
    for (error, says) in [
        (Error::Missing, "not in the cache"),
        (Error::TooLarge, "larger than"),
        (
            Error::Unsupported("its colours".into()),
            "not loaded: its colours",
        ),
        (Error::Invalid("its levels".into()), "invalid: its levels"),
    ] {
        assert!(error.to_string().contains(says), "{error}");
    }
}

/// A cache holding `bytes`, whatever its digest asks for, or nothing.
struct Holding(Option<Vec<u8>>);

impl Source for Holding {
    fn read(&self, _: &AssetRef, limit: usize) -> Option<Vec<u8>> {
        self.0
            .as_ref()
            .map(|bytes| bytes[..bytes.len().min(limit)].to_vec())
    }
}

#[test]
fn a_texture_is_loaded_by_its_digest_from_the_cache_with_its_levels() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        // RGBA with every level, each written as its size lays it out.
        let ktx2 = Ktx2::new(43, 8, 4, rgba(8, 4)).bytes();
        let texture = load(
            &device,
            &queue,
            &Holding(Some(ktx2.clone())),
            Digest::of(&ktx2),
        )
        .unwrap();
        assert_eq!(texture.mip_level_count(), 4);
        assert_eq!(texture.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!((texture.width(), texture.height()), (8, 4));
        // A PNG, one level.
        let (png, _) = png(5, 3);
        let texture = load(
            &device,
            &queue,
            &Holding(Some(png.clone())),
            Digest::of(&png),
        )
        .unwrap();
        assert_eq!((texture.mip_level_count(), texture.width()), (1, 5));
        // ASTC, compressed, where the device samples it, as the headset's does, and refused where it does not.
        let astc = Ktx2::new(
            157,
            8,
            8,
            vec![vec![0; 64], vec![0; 16], vec![0; 16], vec![0; 16]],
        )
        .bytes();
        let loaded = load(
            &device,
            &queue,
            &Holding(Some(astc.clone())),
            Digest::of(&astc),
        );
        assert_eq!(loaded.is_ok(), device.features().contains(ASTC));
        // Not in the cache, or not what its digest names, or too large to read.
        assert_eq!(
            load(&device, &queue, &Holding(None), Digest::of(&png)).unwrap_err(),
            Error::Missing
        );
        assert_eq!(
            load(&device, &queue, &Holding(Some(png)), Digest::of(&ktx2)).unwrap_err(),
            Error::Missing
        );
        let huge = vec![0; BYTES + 10];
        assert_eq!(
            load(
                &device,
                &queue,
                &Holding(Some(huge.clone())),
                Digest::of(&huge)
            )
            .unwrap_err(),
            Error::TooLarge
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
