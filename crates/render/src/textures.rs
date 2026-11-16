// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Textures loaded by digest, as materials name them, from the content store's cache on the device into the GPU: a
//! KTX2 texture of ASTC blocks kept compressed on a device that samples ASTC, as the Quest's does, or of RGBA; and a
//! PNG decoded to RGBA; each with the mipmaps it carries, and no others. Nothing samples them yet: the render paths to
//! come will. See `design/rendering.md`.
use tomori_assets::{AssetRef, Digest, Source};

/// The device features loading textures uses where the adapter has them: sampling ASTC, which the headset's GPU does.
pub const FEATURES: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_ASTC;

/// The most texels a side of a texture, as `tomori-assets` reads a model's.
pub const LARGEST: u32 = 2048;

/// The most bytes a texture is read in.
pub const BYTES: usize = 16 << 20;

/// KTX2's identifier, its first twelve bytes.
const KTX2: [u8; 12] = [
    0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n',
];

/// PNG's signature.
const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// Why a texture is not loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The cache does not have it as its digest says.
    Missing,
    /// It is more than [`BYTES`], or more than [`LARGEST`] texels a side.
    TooLarge,
    /// It is neither KTX2 nor PNG, or of a format or shape this does not load, or the device cannot sample it.
    Unsupported(String),
    /// It is not what it says it is.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Missing => formatter.write_str("the texture is not in the cache"),
            Error::TooLarge => write!(
                formatter,
                "the texture is larger than {BYTES} bytes or {LARGEST} texels a side"
            ),
            Error::Unsupported(why) => write!(formatter, "the texture is not loaded: {why}"),
            Error::Invalid(why) => write!(formatter, "the texture is invalid: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// A texture decoded, ready to upload: its format, its size, and each of its mip levels' bytes, the largest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

/// The texture `bytes` hold, for a device with `features`: KTX2, by its identifier, or PNG, by its signature.
pub fn decode(bytes: &[u8], features: wgpu::Features) -> Result<Decoded, Error> {
    if bytes.len() > BYTES {
        Err(Error::TooLarge)
    } else if bytes.starts_with(&KTX2) {
        ktx2(bytes, features)
    } else if bytes.starts_with(&PNG) {
        png(bytes)
    } else {
        Err(Error::Unsupported("it is neither KTX2 nor PNG".into()))
    }
}

/// A PNG, as RGBA in sRGB, one level.
fn png(bytes: &[u8]) -> Result<Decoded, Error> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(LARGEST);
    limits.max_image_height = Some(LARGEST);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| match error {
        image::ImageError::Limits(_) => Error::TooLarge,
        error => Error::Invalid(error.to_string()),
    })?;
    let image = image.to_rgba8();
    Ok(Decoded {
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        width: image.width(),
        height: image.height(),
        levels: vec![image.into_raw()],
    })
}

/// The format of KTX2's `vk_format`, a Vulkan format: RGBA of eight bits a channel, or ASTC's blocks, in linear or sRGB.
fn format(vk_format: u32) -> Option<wgpu::TextureFormat> {
    use wgpu::{AstcBlock, AstcChannel, TextureFormat};
    const BLOCKS: [AstcBlock; 14] = [
        AstcBlock::B4x4,
        AstcBlock::B5x4,
        AstcBlock::B5x5,
        AstcBlock::B6x5,
        AstcBlock::B6x6,
        AstcBlock::B8x5,
        AstcBlock::B8x6,
        AstcBlock::B8x8,
        AstcBlock::B10x5,
        AstcBlock::B10x6,
        AstcBlock::B10x8,
        AstcBlock::B10x10,
        AstcBlock::B12x10,
        AstcBlock::B12x12,
    ];
    match vk_format {
        // VK_FORMAT_R8G8B8A8_UNORM and VK_FORMAT_R8G8B8A8_SRGB.
        37 => Some(TextureFormat::Rgba8Unorm),
        43 => Some(TextureFormat::Rgba8UnormSrgb),
        // VK_FORMAT_ASTC_4x4_UNORM_BLOCK to VK_FORMAT_ASTC_12x12_SRGB_BLOCK, each block's linear then its sRGB.
        157..=184 => {
            let index = vk_format - 157;
            Some(TextureFormat::Astc {
                block: BLOCKS[(index / 2) as usize],
                channel: if index.is_multiple_of(2) {
                    AstcChannel::Unorm
                } else {
                    AstcChannel::UnormSrgb
                },
            })
        }
        _ => None,
    }
}

/// How a level of `format` `width` by `height` is laid out: the bytes of each row of its blocks, and how many rows.
pub fn layout(format: wgpu::TextureFormat, width: u32, height: u32) -> (u32, u32) {
    let (block_width, block_height) = format.block_dimensions();
    let block = format.block_copy_size(None).unwrap_or(4);
    (
        width.div_ceil(block_width) * block,
        height.div_ceil(block_height),
    )
}

/// A KTX2 texture: one 2D image, not supercompressed, of a [`format`] the device samples, with its levels as it gives
/// them, each where it says and as long as its size makes it.
fn ktx2(bytes: &[u8], features: wgpu::Features) -> Result<Decoded, Error> {
    let invalid = |why: &str| Error::Invalid(why.into());
    // Its header, the identifier and 68 bytes after it, before its index of levels.
    let header = bytes
        .get(..80)
        .ok_or_else(|| invalid("its header is cut short"))?;
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().expect("four bytes"));
    let vk_format = word(12);
    let (width, height, depth) = (word(20), word(24), word(28));
    let (layers, faces, count, supercompression) = (word(32), word(36), word(40), word(44));
    let format = format(vk_format)
        .ok_or_else(|| Error::Unsupported(format!("its Vulkan format {vk_format}")))?;
    if depth != 0 || layers != 0 || faces != 1 {
        return Err(Error::Unsupported("it is not one 2D image".into()));
    }
    if supercompression != 0 {
        return Err(Error::Unsupported("it is supercompressed".into()));
    }
    if width == 0 || height == 0 {
        return Err(invalid("it has no texels"));
    }
    if width > LARGEST || height > LARGEST {
        return Err(Error::TooLarge);
    }
    let (block_width, block_height) = format.block_dimensions();
    if !width.is_multiple_of(block_width) || !height.is_multiple_of(block_height) {
        return Err(Error::Unsupported("its size is not whole blocks".into()));
    }
    if !features.contains(format.required_features()) {
        return Err(Error::Unsupported(format!(
            "the device cannot sample {format:?}"
        )));
    }
    // Zero levels asks for mipmaps to be made, which loading does not: its one level is drawn.
    let count = count.max(1);
    if count > 32 - width.max(height).leading_zeros() {
        return Err(invalid("it has more levels than its size halves to"));
    }
    // Each level's entry: its offset, its length and its length uncompressed, each eight bytes.
    let index = bytes
        .get(80..80 + count as usize * 24)
        .ok_or_else(|| invalid("its index of levels is cut short"))?;
    let long = |at: usize| u64::from_le_bytes(index[at..at + 8].try_into().expect("eight bytes"));
    let levels = (0..count)
        .map(|level| {
            let (offset, length) = (long(level as usize * 24), long(level as usize * 24 + 8));
            let (row, rows) = layout(format, (width >> level).max(1), (height >> level).max(1));
            if length != u64::from(row) * u64::from(rows) {
                return Err(invalid("a level's length is not its size's"));
            }
            let end = offset.checked_add(length);
            usize::try_from(offset)
                .ok()
                .zip(end.and_then(|end| usize::try_from(end).ok()))
                .and_then(|(start, end)| bytes.get(start..end))
                .map(<[u8]>::to_vec)
                .ok_or_else(|| invalid("a level lies outside it"))
        })
        .collect::<Result<_, _>>()?;
    Ok(Decoded {
        format,
        width,
        height,
        levels,
    })
}

/// Upload `decoded` as a texture to sample, with each of its levels.
pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, decoded: &Decoded) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Material texture"),
        size: wgpu::Extent3d {
            width: decoded.width,
            height: decoded.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: decoded.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: decoded.format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, bytes) in decoded.levels.iter().enumerate() {
        let size = texture
            .size()
            .mip_level_size(level as u32, wgpu::TextureDimension::D2);
        let (row, rows) = layout(decoded.format, size.width, size.height);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(rows),
            },
            size.physical_size(decoded.format),
        );
    }
    texture
}

/// The texture `digest` names, read from `cache`, the content store's cache on the device, decoded for `device` and
/// uploaded with its mipmaps; or why not.
pub fn load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cache: &(impl Source + ?Sized),
    digest: Digest,
) -> Result<wgpu::Texture, Error> {
    let asset = AssetRef::new("content", 0, digest.to_string(), digest);
    let bytes = cache
        .read(&asset, BYTES + 1)
        .filter(|bytes| bytes.len() > BYTES || asset.matches(bytes))
        .ok_or(Error::Missing)?;
    Ok(upload(device, queue, &decode(&bytes, device.features())?))
}

#[cfg(test)]
mod tests;
