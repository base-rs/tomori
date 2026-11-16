// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's surface textures: packed earth for the yard and the trail, stone for the river's stones and the
//! waterfall rock, and bark for the pines', cabbage trees' and podocarps' trunks, each a base colour and a normal map
//! made by `tomori-texture-gen` from a recipe here, in ASTC, which the Quest samples, and in BC7, which desktops do, as
//! KTX2 with every mip level. Each is named here by the digest of its bytes, so that a client looks for exactly what its
//! server serves, and the materials of the farmyard's models of the same names, [`crate::props::EARTH`],
//! [`crate::props::STONE`] and [`crate::props::BARK`], lay them on their faces where a client has them. The server
//! generates them, with the `side` feature, as it first reads them, [`bundle`]. See `design/farmyard.md` and
//! `design/rendering.md`.
use tomori_assets::{AssetRef, Digest};

/// One of the farmyard's surfaces: the material it is for, by name, the tile it is generated as, how a client lays it on
/// faces, its base colour's mean, and the digest of each of its textures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    /// The material it is for, `earth`, `stone` or `bark`, which names its recipe's surface too.
    pub name: &'static str,
    /// Its tile's texels a side, and the seed it is made from.
    pub size: u32,
    pub seed: u32,
    /// How many metres its tile is across, laid on faces, and how far each piece's is turned at most, in radians.
    pub metres: f32,
    pub turn: f32,
    /// Its base colour's mean, linear RGB, as generating it says.
    pub flat: [f32; 3],
    /// The digests of its base colour in ASTC and in BC7, then of its normal map in ASTC and in BC7.
    digests: [&'static str; 4],
}

/// Which of a surface's textures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Map {
    /// Its base colour, in sRGB.
    Colour,
    /// Its tangent-space normal map.
    Normal,
}

/// The compressed forms each texture is served in, by the names a texture's asset ends in: ASTC, which the Quest
/// samples, and BC7, which desktops do.
pub const FORMS: [&str; 2] = ["astc", "bc7"];

/// The farmyard's surfaces. Earth's tile is 2 m across, about 2 mm a texel, laid alike on every chunk of the ground so
/// that they meet in it; stone's 1.5 m, each stone's turned its own way; bark's 1 m, 512 texels, as trunks are thin and
/// seen from further, each turned a little.
pub const SURFACES: [Surface; 3] = [
    Surface {
        name: "earth",
        size: 1024,
        seed: 1,
        metres: 2.0,
        turn: 0.0,
        flat: [0.24340007, 0.16602688, 0.1033788],
        digests: [
            "744855a3bcc05d22cf93482262f84c044d8af7a12399050ea2cb820ee166a6fe",
            "18fadb74b12f00cbbaec15f78abbd0aff4e3db9f407fb6b363c0e66cc9a59d5d",
            "bf798f99a55ef1fbc224de647f6b37651d9479bd003a7c07b9ded3ddfffb4215",
            "2bce927f3bab3203c0c0658a59b4ab242aca73bd95284fd566d92efc17a1c66a",
        ],
    },
    Surface {
        name: "stone",
        size: 1024,
        seed: 2,
        metres: 1.5,
        turn: std::f32::consts::PI,
        flat: [0.24344233, 0.24119738, 0.23155196],
        digests: [
            "f3f2570a9bb156f1114b8beb941c4f22f44c213ad88ff829d6821200b480571d",
            "74d3ee575dba1a97d00d46f7ad5406dea65b6912a8a162add3394f420d5e14df",
            "2421908977a8db5e18420a648d6473b63c052ee31a6d99f01b32ba1b35987e2e",
            "44c287cd3b610de84bdcb33950cd54911fecd64804728d530c70d1bb6d90a8f2",
        ],
    },
    Surface {
        name: "bark",
        size: 512,
        seed: 3,
        metres: 1.0,
        turn: 0.15,
        flat: [0.1385398, 0.09594671, 0.07041724],
        digests: [
            "50f8331ab98581d68ec009cdb08a74869e670808d0f510dab2c8da3f8f913c0d",
            "36a951a1ca70f99febefee2546835e540be915dc9a3b7218bc1afbacff788437",
            "481ad5ba99da1be31c5966a211d55955b772377ee30bd23bce586db657d817db",
            "7dc07e7f82ea758178dfa2c9d81f72b2b2b66bf00a80d5d1637cb49e27bd2e48",
        ],
    },
];

impl Surface {
    /// The surface for the material named `name`, if it has one.
    pub fn named(name: &str) -> Option<&'static Surface> {
        SURFACES.iter().find(|surface| surface.name == name)
    }

    /// Its texture `map` in the compressed form `form`, `astc` or `bc7`, as the farmyard serves it, named
    /// `textures/NAME-MAP.FORM.ktx2`; none for another form.
    pub fn asset(&self, map: Map, form: &str) -> Option<AssetRef> {
        let at = FORMS.iter().position(|known| *known == form)?;
        let (index, map_name) = match map {
            Map::Colour => (at, "colour"),
            Map::Normal => (2 + at, "normal"),
        };
        Some(AssetRef::new(
            crate::ID,
            crate::VERSION,
            format!("textures/{}-{map_name}.{form}.ktx2", self.name),
            Digest::parse(self.digests[index]).expect("a surface's digest is written as one"),
        ))
    }
}

/// Every texture the farmyard serves, each surface's base colour and normal map in each form.
pub fn assets() -> impl Iterator<Item = AssetRef> {
    SURFACES.iter().flat_map(|surface| {
        [Map::Colour, Map::Normal].into_iter().flat_map(move |map| {
            FORMS
                .iter()
                .filter_map(move |form| surface.asset(map, form))
        })
    })
}

/// Generating the farmyard's textures, which only a server does.
#[cfg(feature = "side")]
mod generated {
    use std::sync::OnceLock;

    use tomori_assets::{AssetRef, Digest, Library, Source};
    use tomori_texture_gen::{Format, Recipe};

    use super::{Map, SURFACES, Surface};

    /// A surface's recipe.
    pub(super) fn recipe(surface: &Surface) -> Recipe {
        let kind = tomori_texture_gen::Surface::ALL
            .into_iter()
            .find(|kind| kind.name() == surface.name)
            .expect("each of the farmyard's surfaces is one the generator makes");
        Recipe::new(kind, surface.size, surface.seed)
    }

    /// One of a surface's textures: which, in which form, and its bytes.
    pub(super) type Texture = (Map, &'static str, Vec<u8>);

    /// A surface's textures, and its base colour's mean.
    pub(super) fn generate(surface: &Surface) -> (Vec<Texture>, [f32; 3]) {
        let set = recipe(surface).generate();
        let textures = [
            (Map::Colour, &set.color, true),
            (Map::Normal, &set.normal, false),
        ]
        .into_iter()
        .flat_map(|(map, chain, srgb)| {
            [(Format::Astc, "astc"), (Format::Bc7, "bc7")]
                .into_iter()
                .map(move |(format, form)| (map, form, chain.ktx2(format, srgb)))
        })
        .collect();
        (textures, set.flat)
    }

    /// The farmyard's textures, each by its digest, generated once, the first time they are asked for, and the log
    /// says how long that took.
    pub fn textures() -> &'static [(Digest, Vec<u8>)] {
        static TEXTURES: OnceLock<Vec<(Digest, Vec<u8>)>> = OnceLock::new();
        TEXTURES.get_or_init(|| {
            let started = std::time::Instant::now();
            let textures: Vec<(Digest, Vec<u8>)> = SURFACES
                .iter()
                .flat_map(|surface| generate(surface).0)
                .map(|(_, _, bytes)| (Digest::of(&bytes), bytes))
                .collect();
            log::info!(
                "Generated the farmyard's {} textures, {} bytes, in {:.0} ms",
                textures.len(),
                textures.iter().map(|(_, bytes)| bytes.len()).sum::<usize>(),
                started.elapsed().as_secs_f64() * 1e3
            );
            textures
        })
    }

    /// The farmyard's textures, as a source of assets.
    pub struct Generated;

    impl Source for Generated {
        fn read(&self, asset: &AssetRef, limit: usize) -> Option<Vec<u8>> {
            textures()
                .iter()
                .find(|(digest, _)| *digest == asset.digest())
                .map(|(_, bytes)| bytes[..bytes.len().min(limit)].to_vec())
        }
    }

    /// Add the farmyard's textures to the program's asset library, generated the first time one is read, as the server
    /// does as it registers the farmyard, so that its content store serves them.
    pub fn bundle() {
        static BUNDLED: OnceLock<()> = OnceLock::new();
        BUNDLED.get_or_init(|| Library::shared().add(Generated));
    }
}

#[cfg(feature = "side")]
pub use generated::{Generated, bundle, textures};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_material_of_a_surface_names_its_textures_in_each_form() {
        let names: Vec<String> = assets().map(|asset| asset.name().to_owned()).collect();
        assert_eq!(names.len(), 12);
        assert_eq!(names[0], "textures/earth-colour.astc.ktx2");
        assert_eq!(names[3], "textures/earth-normal.bc7.ktx2");
        assert_eq!(names[11], "textures/bark-normal.bc7.ktx2");
        // Each of its own bytes.
        let mut digests: Vec<Digest> = assets().map(|asset| asset.digest()).collect();
        digests.sort_by_key(|digest| digest.to_string());
        digests.dedup();
        assert_eq!(digests.len(), 12);
        assert_eq!(
            Surface::named("stone").map(|surface| surface.metres),
            Some(1.5)
        );
        assert_eq!(Surface::named("plain"), None);
        assert_eq!(SURFACES[0].asset(Map::Colour, "etc2"), None);
        // Each a material of the farmyard's models, of the same name.
        let materials = crate::props::materials();
        for (surface, material) in
            SURFACES
                .iter()
                .zip([crate::props::EARTH, crate::props::STONE, crate::props::BARK])
        {
            assert_eq!(materials[material].name.as_deref(), Some(surface.name));
        }
    }

    /// What the server generates is exactly what the farmyard names, by digest, and its base colour's mean is as said.
    #[cfg(feature = "side")]
    #[test]
    fn the_textures_generated_are_those_their_digests_name() {
        let mut found = Vec::new();
        for surface in &SURFACES {
            let (textures, flat) = generated::generate(surface);
            for (map, form, bytes) in textures {
                let asset = surface.asset(map, form).unwrap();
                assert!(bytes.starts_with(&tomori_texture_gen::KTX2));
                assert!(bytes.len() as u64 <= tomori_protocol::ContentKind::Texture.most_bytes());
                found.push(format!("{} {}", asset.name(), Digest::of(&bytes)));
            }
            found.push(format!("{} flat {flat:?}", surface.name));
        }
        let expected: Vec<String> = SURFACES
            .iter()
            .flat_map(|surface| {
                [Map::Colour, Map::Normal]
                    .into_iter()
                    .flat_map(move |map| {
                        FORMS.iter().map(move |form| {
                            let asset = surface.asset(map, form).unwrap();
                            format!("{} {}", asset.name(), asset.digest())
                        })
                    })
                    .chain([format!("{} flat {:?}", surface.name, surface.flat)])
            })
            .collect();
        assert_eq!(found, expected, "{found:#?}");
        // And the library reads them, once bundled.
        bundle();
        let asset = SURFACES[2].asset(Map::Normal, "astc").unwrap();
        assert!(tomori_assets::Library::shared().bytes(&asset).is_ok());
    }
}
