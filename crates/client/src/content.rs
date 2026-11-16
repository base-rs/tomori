// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Fetching content by digest beside the connection: each of the environments' assets the server offers that the
//! cache does not keep, one at a time, with `GET /content/DIGEST` on a connection of its own from the client's pool,
//! presenting the connection's credential and its session ticket; resumed where a download that stopped left off, and
//! fetched again whole, once, if what it resumed was not the asset; refused unless it is of the type offered, no
//! longer than offered and starts as its kind must; and kept only once it is what its digest says. See
//! `design/clients.md`.
use std::path::PathBuf;

use base_http::body::{BodyFrame, ReadableBody};
use base_http::{Fields, Path, Request};
use base_http_client::Client as HttpClient;
use base_scheduler::Cancellation;
use tomori_assets::{Cache, Digest};
use tomori_protocol::{CONTENT_PATH, OfferedAsset, SESSION_FIELD};

/// The most bytes a client's cache keeps by default: room for several environments' assets, and little of a headset's
/// storage.
pub const CACHE_BYTES: u64 = 256 << 20;

/// What a server offered this connection: its session ticket and the assets it may fetch.
#[derive(Debug)]
pub(crate) struct Offer {
    pub session: String,
    pub assets: Vec<OfferedAsset>,
}

/// The folder a desktop keeps its cache of content in: `tomori/content` in the user's cache folder, `XDG_CACHE_HOME`
/// where it is set to an absolute path, or else `~/Library/Caches` on macOS and `~/.cache` elsewhere, or `None` without
/// a home.
pub fn cache_folder() -> Option<PathBuf> {
    cache_folder_in(
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        cfg!(target_os = "macos"),
    )
}

fn cache_folder_in(xdg: Option<PathBuf>, home: Option<PathBuf>, macos: bool) -> Option<PathBuf> {
    let caches = match (xdg.filter(|xdg| xdg.is_absolute()), home) {
        (Some(xdg), _) => xdg,
        (None, Some(home)) if macos => home.join("Library/Caches"),
        (None, Some(home)) => home.join(".cache"),
        (None, None) => return None,
    };
    Some(caches.join("tomori/content"))
}

/// Whether a device sampling textures of the compressed form `textures`, if it says, wants `asset`: anything but a
/// texture named for another form, `NAME.FORM.ktx2`, of which a server offers each form and a device draws one.
pub(crate) fn wanted(asset: &OfferedAsset, textures: Option<&str>) -> bool {
    let form = asset
        .name
        .strip_suffix(".ktx2")
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, form)| form);
    match (asset.kind, form, textures) {
        (tomori_protocol::ContentKind::Texture, Some(form), Some(textures)) => form == textures,
        _ => true,
    }
}

/// Fetch into `cache` each asset `offer` offers that it does not keep and the device wants, as [`wanted`] says of a
/// device sampling `textures`, presenting `authorization`, if any, and the offer's ticket, until `lifetime`, the
/// connection's, ends. Each that cannot be fetched is skipped, saying why; the next connection tries it again.
pub(crate) async fn fetch_offered(
    client: &HttpClient<'_>,
    cache: &Cache,
    authorization: Option<&str>,
    offer: Offer,
    textures: Option<&str>,
    lifetime: &Cancellation,
) {
    // What was kept of a download that stopped is resumed first, as the cache keeps only one download at a time.
    let mut assets: Vec<&OfferedAsset> = offer
        .assets
        .iter()
        .filter(|asset| wanted(asset, textures))
        .collect();
    assets.sort_by_key(|asset| {
        Digest::parse(&asset.digest).is_none_or(|digest| cache.partial(&digest) == 0)
    });
    for asset in assets {
        let path = format!("{}/{}", asset.environment, asset.name);
        let Some(digest) =
            Digest::parse(&asset.digest).filter(|digest| digest.to_string() == asset.digest)
        else {
            log::warn!("Did not fetch {path}: its digest is not written as one.");
            continue;
        };
        if asset.length > asset.kind.most_bytes() || asset.length > cache.most() {
            log::warn!(
                "Did not fetch {path}: it is {} bytes, more than a {} may be or the cache keeps.",
                asset.length,
                asset.kind.content_type()
            );
            continue;
        }
        if cache.contains(&digest) {
            continue;
        }
        let fetching = Fetching {
            client,
            cache,
            authorization,
            session: &offer.session,
            asset,
            digest,
            lifetime,
        };
        // Resumed where a download that stopped left off, and fetched again whole, once, if that fails, as when what was
        // kept of it was not the asset's.
        let offset = Some(cache.partial(&digest))
            .filter(|kept| *kept < asset.length)
            .unwrap_or(0);
        let mut fetched = fetching.fetch(offset).await;
        if fetched.is_err() && offset > 0 {
            fetched = fetching.fetch(0).await;
        }
        match fetched {
            Ok(()) => log::info!("Fetched {path}, {} bytes, by its digest.", asset.length),
            Err(error) => log::warn!("Could not fetch {path}: {error}."),
        }
    }
}

/// One asset being fetched.
struct Fetching<'a, 'scope> {
    client: &'a HttpClient<'scope>,
    cache: &'a Cache,
    authorization: Option<&'a str>,
    session: &'a str,
    asset: &'a OfferedAsset,
    digest: Digest,
    lifetime: &'a Cancellation,
}

impl Fetching<'_, '_> {
    /// Fetch it from `offset`, after what was kept of an earlier download, or from the start, or say why not.
    async fn fetch(&self, offset: u64) -> Result<(), String> {
        let mut request = Request::get(Path::from(format!("{CONTENT_PATH}{}", self.asset.digest)));
        let mut fields = vec![(SESSION_FIELD, self.session.to_owned())];
        if let Some(authorization) = self.authorization {
            fields.push(("authorization", authorization.to_owned()));
        }
        if offset > 0 {
            fields.push(("range", format!("bytes={offset}-")));
        }
        for (name, value) in fields {
            // A ticket or token that is not a valid field value is not presented, and the server refuses the request.
            let _ = request.fields.set(name, value);
        }
        let response = self
            .client
            .call(request, self.lifetime)
            .await
            .map_err(text)?;
        let status = response.status.as_u16();
        let range = field(&response.fields, "content-range").and_then(|range| range_start(&range));
        let start = match (status, range) {
            (200, _) => 0,
            (206, Some(start)) if start == offset => offset,
            _ => return Err(format!("the server answered {status}")),
        };
        let content_type = self.asset.kind.content_type();
        if field(&response.fields, "content-type").as_deref() != Some(content_type) {
            return Err(format!("it is not {content_type}"));
        }
        let mut writer = self.cache.writer(&self.digest, start).map_err(text)?;
        let mut body = response.body;
        while let BodyFrame::Data { data, end } = body.read(self.lifetime).await.map_err(text)? {
            let wrote = if writer.written() + data.len() as u64 > self.asset.length {
                Err("it is longer than offered".to_owned())
            } else {
                writer.write(&data).map_err(text)
            };
            wrote?;
            if end {
                break;
            }
        }
        // Its digest is the server's word for what it is, and its signature checked too, as the server checks it.
        if !self.asset.kind.recognises(writer.head()) {
            return Err(format!("it does not start as {content_type} must"));
        }
        writer.finish().map_err(text)
    }
}

/// What went wrong, as the log says it.
fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The first value of the field `name`, if it is text.
fn field(fields: &Fields, name: &str) -> Option<String> {
    fields
        .values(name)
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(str::to_owned)
}

/// Where a `Content-Range` of `bytes START-END/LENGTH` starts.
fn range_start(field: &str) -> Option<u64> {
    field
        .strip_prefix("bytes ")?
        .split_once('-')?
        .0
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_keeps_its_cache_in_its_user_s_cache_folder() {
        let home = Some(PathBuf::from("/home/ada"));
        let xdg = Some(PathBuf::from("/var/cache/ada"));
        assert_eq!(
            cache_folder_in(xdg.clone(), home.clone(), false),
            Some(PathBuf::from("/var/cache/ada/tomori/content"))
        );
        assert_eq!(
            cache_folder_in(None, home.clone(), false),
            Some(PathBuf::from("/home/ada/.cache/tomori/content"))
        );
        // A relative folder is not one, as the XDG specification says.
        assert_eq!(
            cache_folder_in(Some(PathBuf::from("cache")), home.clone(), false),
            Some(PathBuf::from("/home/ada/.cache/tomori/content"))
        );
        assert_eq!(
            cache_folder_in(None, home, true),
            Some(PathBuf::from("/home/ada/Library/Caches/tomori/content"))
        );
        assert_eq!(
            cache_folder_in(xdg, None, true),
            Some(PathBuf::from("/var/cache/ada/tomori/content"))
        );
        assert_eq!(cache_folder_in(None, None, false), None);
        // Wherever this runs, it is somewhere or nowhere.
        let _ = cache_folder();
    }

    #[test]
    fn a_texture_is_fetched_only_in_the_form_the_device_samples() {
        let asset = |name: &str, kind| OfferedAsset {
            environment: "farmyard".into(),
            version: 1,
            name: name.into(),
            digest: "0".repeat(64),
            length: 1,
            kind,
            licence: None,
        };
        let texture = tomori_protocol::ContentKind::Texture;
        let (astc, bc7) = (
            asset("textures/earth-colour.astc.ktx2", texture),
            asset("textures/earth-colour.bc7.ktx2", texture),
        );
        assert!(wanted(&astc, Some("astc")) && !wanted(&bc7, Some("astc")));
        assert!(!wanted(&astc, Some("bc7")) && wanted(&bc7, Some("bc7")));
        // A device that does not say fetches every form.
        assert!(wanted(&astc, None) && wanted(&bc7, None));
        // A texture named for no form, and what is not a texture, whatever its name.
        assert!(wanted(&asset("moss.ktx2", texture), Some("astc")));
        assert!(wanted(&asset("moss.png", texture), Some("astc")));
        assert!(wanted(
            &asset("chicken.bc7.ktx2", tomori_protocol::ContentKind::Model),
            Some("astc")
        ));
    }

    #[test]
    fn a_content_range_says_where_it_starts() {
        assert_eq!(range_start("bytes 10-19/20"), Some(10));
        assert_eq!(range_start("bytes */20"), None);
        assert_eq!(range_start("items 1-2/3"), None);
        assert_eq!(range_start("bytes 10"), None);
    }
}
