// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Textures loaded by digest, as materials name them, from the content store's cache on the device into the GPU: a
//! KTX2 texture of ASTC blocks kept compressed on a device that samples ASTC, as the Quest's does, of BC7 blocks on one
//! that samples BC7, as desktops do, or of RGBA; and a PNG decoded to RGBA; each with the mipmaps it carries, or, for
//! RGBA that carries one level, made from it in linear light as it is loaded. [`Resident`] keeps a scenery's textures
//! as they arrive. See `design/rendering.md`.
use tomori_assets::{AssetRef, Digest, Library, Source};

/// The device features loading textures uses where the adapter has them: sampling ASTC, which the headset's GPU does,
/// and BC7, which desktop GPUs do.
pub const FEATURES: wgpu::Features =
    wgpu::Features::TEXTURE_COMPRESSION_ASTC.union(wgpu::Features::TEXTURE_COMPRESSION_BC);

/// A block-compressed form a texture is served in, of which a device loads the one it samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    /// ASTC's blocks, which the Quest's GPU samples, and most desktop GPUs do not.
    Astc,
    /// BC7, which desktop GPUs sample, and the Quest's does not.
    Bc7,
}

impl Compression {
    /// The form a device with `features` samples, ASTC first, as it is what a headset samples; none if it samples
    /// neither.
    pub fn sampled(features: wgpu::Features) -> Option<Self> {
        if features.contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC) {
            Some(Self::Astc)
        } else if features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            Some(Self::Bc7)
        } else {
            None
        }
    }

    /// Its name, as a texture's asset names it: `astc` or `bc7`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Astc => "astc",
            Self::Bc7 => "bc7",
        }
    }
}

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

/// A PNG, as RGBA in sRGB, with its mipmaps made from it.
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
    let (width, height) = image.dimensions();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    Ok(Decoded {
        format,
        width,
        height,
        levels: mipmaps(format, width, height, image.into_raw()),
    })
}

/// Every level of an RGBA texture of `format`, `width` by `height`, from its first, `texels`: each next half the last
/// down to one texel, each texel the mean of up to four of the last in linear light, its alpha as it is, so that what is
/// far away does not shimmer.
pub fn mipmaps(
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    texels: Vec<u8>,
) -> Vec<Vec<u8>> {
    let srgb = format.is_srgb();
    let linear: [f32; 256] = std::array::from_fn(|byte| {
        let value = byte as f32 / 255.0;
        if !srgb {
            value
        } else if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    let encode = |value: f32, channel: usize| {
        let value = value.clamp(0.0, 1.0);
        let encoded = if !srgb || channel == 3 {
            value
        } else if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    };
    let mut levels = vec![texels];
    let (mut width, mut height) = (width as usize, height as usize);
    while width > 1 || height > 1 {
        let last = levels.last().expect("the first level");
        let (next_width, next_height) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = Vec::with_capacity(next_width * next_height * 4);
        for y in 0..next_height {
            for x in 0..next_width {
                for channel in 0..4 {
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let at = ((y * 2 + dy).min(height - 1) * width
                            + (x * 2 + dx).min(width - 1))
                            * 4
                            + channel;
                        sum += if channel == 3 {
                            f32::from(last[at]) / 255.0
                        } else {
                            linear[usize::from(last[at])]
                        };
                    }
                    next.push(encode(sum / 4.0, channel));
                }
            }
        }
        levels.push(next);
        (width, height) = (next_width, next_height);
    }
    levels
}

/// The format of KTX2's `vk_format`, a Vulkan format: RGBA of eight bits a channel, BC7's blocks, or ASTC's, in linear
/// or sRGB.
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
        // VK_FORMAT_BC7_UNORM_BLOCK and VK_FORMAT_BC7_SRGB_BLOCK.
        145 => Some(TextureFormat::Bc7RgbaUnorm),
        146 => Some(TextureFormat::Bc7RgbaUnormSrgb),
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
/// them, each where it says and as long as its size makes it; RGBA of one level, asking for its mipmaps to be made or
/// not, with them made.
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
    // Zero levels asks for mipmaps to be made, which are made below for RGBA, as they are for one level of it.
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
        .collect::<Result<Vec<_>, _>>()?;
    let levels = match levels.as_slice() {
        [level] if !format.is_compressed() => mipmaps(format, width, height, level.clone()),
        _ => levels,
    };
    Ok(Decoded {
        format,
        width,
        height,
        levels,
    })
}

/// Upload `decoded` as a texture to sample, with each of its levels, saying so in the log for one that is compressed,
/// so that a device's log shows what it uploaded as ASTC or BC7.
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
    if decoded.format.is_compressed() {
        log::info!("{}", uploaded(decoded));
    }
    texture
}

/// What the log says of `decoded` as it is uploaded: its format, its size, its levels and its bytes.
fn uploaded(decoded: &Decoded) -> String {
    let (block_width, block_height) = decoded.format.block_dimensions();
    let kind = match decoded.format {
        wgpu::TextureFormat::Astc { .. } => format!("ASTC {block_width}×{block_height}"),
        wgpu::TextureFormat::Bc7RgbaUnorm | wgpu::TextureFormat::Bc7RgbaUnormSrgb => {
            "BC7".to_owned()
        }
        format => format!("{format:?}"),
    };
    format!(
        "Uploaded a texture of {kind}{}, {}×{}, {} mip levels, {} bytes",
        if decoded.format.is_srgb() {
            " in sRGB"
        } else {
            ""
        },
        decoded.width,
        decoded.height,
        decoded.levels.len(),
        decoded.levels.iter().map(Vec::len).sum::<usize>()
    )
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

/// How many frames a texture that no source has yet is looked for again after: two seconds at 72 Hz.
pub const RETRY_FRAMES: u32 = 144;

/// What became of one of a scenery's textures.
#[derive(Debug)]
enum State {
    /// No source has it yet: looked for again in this many frames; and whether the log said so.
    Waiting(u32, bool),
    Loaded(wgpu::TextureView),
    /// A source has it, but it cannot be loaded, as on a device that cannot sample its format: never looked for again.
    Refused,
}

/// The textures a scenery draws, by asset, loaded from the program's library as they arrive in its sources, such as
/// the client's cache of what its server serves: at most one a frame, each that no source has yet looked for again every
/// [`RETRY_FRAMES`], and one that cannot be loaded never again. Until one is loaded, what it is for is drawn without it.
/// A frame once each is loaded or refused, or with none due, allocates nothing.
pub struct Resident {
    textures: Vec<(AssetRef, State)>,
}

impl Resident {
    /// `assets`, none loaded, each to be looked for at the next [`Self::poll`] in turn.
    pub fn new(assets: impl IntoIterator<Item = AssetRef>) -> Self {
        Self {
            textures: assets
                .into_iter()
                .map(|asset| (asset, State::Waiting(0, false)))
                .collect(),
        }
    }

    /// Once a frame: look for the first texture due, in `library`, and load it for `device`, saying whether it loaded
    /// one; refused, the log says why, and missing, the log says so once.
    pub fn poll(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, library: &Library) -> bool {
        let mut due = None;
        for (index, (_, state)) in self.textures.iter_mut().enumerate() {
            if let State::Waiting(frames, _) = state {
                *frames = frames.saturating_sub(1);
                if *frames == 0 && due.is_none() {
                    due = Some(index);
                }
            }
        }
        let Some(index) = due else {
            return false;
        };
        let (asset, state) = &mut self.textures[index];
        let said = matches!(state, State::Waiting(_, true));
        *state = match library.bytes(asset) {
            Err(error) if error.kind() == tomori_assets::ErrorKind::Missing => {
                if !said {
                    log::info!(
                        "Texture {} is not here yet, so what it is for is drawn without it until it is",
                        asset.path()
                    );
                }
                State::Waiting(RETRY_FRAMES, true)
            }
            Err(error) => {
                log::warn!("Texture {} is not loaded: {error}", asset.path());
                State::Refused
            }
            Ok(bytes) => match decode(&bytes, device.features()) {
                Ok(decoded) => {
                    let texture = upload(device, queue, &decoded);
                    State::Loaded(texture.create_view(&wgpu::TextureViewDescriptor::default()))
                }
                Err(error) => {
                    log::warn!("Texture {} is not loaded: {error}", asset.path());
                    State::Refused
                }
            },
        };
        matches!(self.textures[index].1, State::Loaded(_))
    }

    /// The texture `digest` names, if it is loaded.
    pub fn get(&self, digest: Digest) -> Option<&wgpu::TextureView> {
        self.textures.iter().find_map(|(asset, state)| match state {
            State::Loaded(view) if asset.digest() == digest => Some(view),
            _ => None,
        })
    }

    /// Whether each texture is loaded or refused, so that none is looked for again.
    pub fn settled(&self) -> bool {
        !self
            .textures
            .iter()
            .any(|(_, state)| matches!(state, State::Waiting(..)))
    }
}

#[cfg(test)]
mod tests;
