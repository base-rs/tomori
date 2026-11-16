// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The content store: immutable content kept by its SHA-256 digest and served at `GET /content/DIGEST`, of a kind from
//! the protocol's fixed allow-list, within its kind's bound. Today it holds the environments' assets: each environment
//! declares the assets it serves, [`ServedAsset`], when it is registered, and the store reads each from the program's
//! asset library, checked against its digest, kind and size, keeps it in memory, and offers it to every connection that
//! presents the environment at its version. A connection reads only what it is offered, presenting its credential and
//! the session ticket its offer gave it; anything else is answered as missing content is. See `design/content.md`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};

use base_scheduler::Cancellation;

use tomori_assets::{AssetRef, Digest, Library};
use tomori_protocol::{ContentKind, LICENCE_BYTES, OfferedAsset};

use crate::environments::Environments;

/// The most bytes of content the store keeps together, all in memory.
pub const STORE_BYTES: u64 = 512 << 20;

/// The most content requests one connection has open at once; another is answered 503.
pub const OPEN_REQUESTS: usize = 4;

/// The longest name an asset is offered by, in bytes.
pub const NAME_BYTES: usize = 256;

/// The most bytes a content response hands its connection at a time, so that it is written as the client reads it.
pub(crate) const PIECE_BYTES: usize = 64 * 1024;

/// One of an environment's assets that the server serves by digest, as the environment declares it with
/// [`crate::Environment::with_asset`]: the asset, as its code names it, the kind it is, and who made it and under what
/// licence, if it says.
///
/// ```
/// use tomori_assets::{AssetRef, Digest};
/// use tomori_protocol::ContentKind;
/// use tomori_server::ServedAsset;
///
/// let lantern = AssetRef::new("garden", 1, "lantern.glb", Digest::of(b"lantern"));
/// let served = ServedAsset::new(lantern.clone(), ContentKind::Model).with_licence("by Ada, CC BY 4.0");
/// assert_eq!(served.reference(), &lantern);
/// assert_eq!(served.kind(), ContentKind::Model);
/// assert_eq!(served.licence(), Some("by Ada, CC BY 4.0"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedAsset {
    reference: AssetRef,
    kind: ContentKind,
    licence: Option<String>,
}

impl ServedAsset {
    /// `reference`, served as `kind`.
    pub fn new(reference: AssetRef, kind: ContentKind) -> Self {
        Self {
            reference,
            kind,
            licence: None,
        }
    }

    /// The asset, offered with `licence`, a short notice of who made it and under what licence, of at most
    /// [`LICENCE_BYTES`]: an asset with a longer one is not served.
    pub fn with_licence(mut self, licence: impl Into<String>) -> Self {
        self.licence = Some(licence.into());
        self
    }

    pub fn reference(&self) -> &AssetRef {
        &self.reference
    }

    pub fn kind(&self) -> ContentKind {
        self.kind
    }

    pub fn licence(&self) -> Option<&str> {
        self.licence.as_deref()
    }
}

/// Content kept, by digest.
struct Kept {
    bytes: Arc<[u8]>,
    kind: ContentKind,
}

/// An asset offered: what a client is told of it, its digest, and the environment, at its version, that declared it,
/// which a connection must present to read it.
struct Offer {
    asset: OfferedAsset,
    digest: Digest,
    owner: (String, u32),
}

#[derive(Default)]
struct Inner {
    kept: HashMap<Digest, Kept>,
    offers: Vec<Offer>,
}

/// The content the server serves by digest, and the responses sending it. See the [module documentation](self).
#[derive(Default)]
pub(crate) struct ContentStore {
    inner: RwLock<Inner>,
    sending: Mutex<Vec<Weak<Progress>>>,
}

/// How a content response is getting on: when its client last took a piece of it, and what ends its HTTP connection, so
/// that one its client stops reading does not hold the connection for ever.
pub(crate) struct Progress {
    taken: Mutex<Instant>,
    cancellation: Cancellation,
}

impl Progress {
    /// A piece was taken, now.
    pub(crate) fn advanced(&self) {
        *self.taken.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
}

/// Why a content request is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Its ticket names no connection of its principal: 401, as a credential refused at `/connect` is.
    Unbound,
    /// The content is not kept, or not offered to the connection: 404, alike, so that nothing says which.
    Missing,
    /// The connection has [`OPEN_REQUESTS`] open already: 503.
    Busy,
}

/// Content found for a request: its bytes and kind, and the connection's permit to have one more request open, given
/// back when the response is dropped.
pub(crate) struct Served {
    pub bytes: Arc<[u8]>,
    pub kind: ContentKind,
    pub permit: Permit,
    pub progress: Arc<Progress>,
}

/// One of a connection's open content requests, counted until it is dropped.
pub(crate) struct Permit(Arc<AtomicUsize>);

impl Permit {
    /// A permit from `open`, the connection's count of open requests, unless it has [`OPEN_REQUESTS`] already.
    pub(crate) fn take(open: &Arc<AtomicUsize>) -> Option<Self> {
        // Counted first, and given back if that is one too many, so that two requests at once cannot both take the last.
        let permit = Self(open.clone());
        (open.fetch_add(1, Ordering::AcqRel) < OPEN_REQUESTS).then_some(permit)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ContentStore {
    /// Watch a response sent on the HTTP connection that `cancellation` ends, from now.
    pub(crate) fn watch(&self, cancellation: Cancellation) -> Arc<Progress> {
        let progress = Arc::new(Progress {
            taken: Mutex::new(Instant::now()),
            cancellation,
        });
        let mut sending = self.sending.lock().unwrap_or_else(PoisonError::into_inner);
        sending.retain(|watched| watched.strong_count() > 0);
        sending.push(Arc::downgrade(&progress));
        progress
    }

    /// End the HTTP connection of each response whose client has taken nothing of it for `timeout` by `now`, as a
    /// connection whose writes stall is ended. See `design/connection-liveness.md`.
    pub(crate) fn sweep(&self, now: Instant, timeout: Duration) {
        let sending = self.sending.lock().unwrap_or_else(PoisonError::into_inner);
        for progress in sending.iter().filter_map(Weak::upgrade) {
            let taken = *progress
                .taken
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if now.saturating_duration_since(taken) >= timeout {
                log::info!(
                    "Ended a content response whose client took nothing of it for {timeout:?}."
                );
                progress.cancellation.cancel();
            }
        }
    }

    /// Keep and offer the assets `environments` declare, read from `library`, in place of those before: an asset kept
    /// already is not read again, one no environment declares now is let go, and one that cannot be read as its digest,
    /// kind and bounds say is not offered, saying why in the log.
    pub(crate) fn register(&self, environments: &Environments, library: &Library) {
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let mut before = std::mem::take(&mut inner.kept);
        let mut total = 0;
        let mut offers = Vec::new();
        for environment in environments.as_slice() {
            for served in environment.assets() {
                let reference = served.reference();
                let digest = reference.digest();
                let kept = match inner.kept.get(&digest) {
                    Some(kept) => Ok(kept.kind),
                    None => before
                        .remove(&digest)
                        .map(Ok)
                        .unwrap_or_else(|| read(served, library))
                        .and_then(|kept| {
                            let length = kept.bytes.len() as u64;
                            if total + length > STORE_BYTES {
                                return Err(format!(
                                    "the content store would hold more than {STORE_BYTES} bytes"
                                ));
                            }
                            total += length;
                            let kind = kept.kind;
                            inner.kept.insert(digest, kept);
                            Ok(kind)
                        }),
                };
                let offered = kept.and_then(|kind| {
                    if kind != served.kind() {
                        return Err(format!("it is kept as {} already", kind.content_type()));
                    }
                    offer(served, environment, inner.kept[&digest].bytes.len() as u64)
                });
                match offered {
                    Ok(offer) => offers.push(offer),
                    Err(error) => log::warn!(
                        "The asset {} of the environment {} is not served: {error}.",
                        reference.path(),
                        environment.id()
                    ),
                }
            }
        }
        inner.offers = offers;
    }

    /// The assets offered to a connection to which `presentable` are offered, at most [`tomori_protocol::ASSET_LIMIT`].
    pub(crate) fn offered(&self, presentable: &Environments) -> Vec<OfferedAsset> {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        inner
            .offers
            .iter()
            .filter(|offer| owned_by(offer, presentable))
            .map(|offer| offer.asset.clone())
            .take(tomori_protocol::ASSET_LIMIT)
            .collect()
    }

    /// The content `digest`, and its kind, if it is kept and offered to a connection to which `presentable` are
    /// offered.
    pub(crate) fn find(
        &self,
        digest: &Digest,
        presentable: &Environments,
    ) -> Option<(Arc<[u8]>, ContentKind)> {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        inner
            .offers
            .iter()
            .any(|offer| offer.digest == *digest && owned_by(offer, presentable))
            .then(|| &inner.kept[digest])
            .map(|kept| (kept.bytes.clone(), kept.kind))
    }
}

/// Whether `offer` is of an environment among `presentable` at its version.
fn owned_by(offer: &Offer, presentable: &Environments) -> bool {
    let (id, version) = &offer.owner;
    presentable.version(id) == Some(*version)
}

/// What `environment` offers of `served`, `length` bytes long, unless its name or licence is too long to offer.
fn offer(
    served: &ServedAsset,
    environment: &crate::Environment,
    length: u64,
) -> Result<Offer, String> {
    let reference = served.reference();
    if reference.name().is_empty() || reference.name().len() > NAME_BYTES {
        return Err(format!(
            "its name is empty or longer than {NAME_BYTES} bytes"
        ));
    }
    if served
        .licence()
        .is_some_and(|licence| licence.len() > LICENCE_BYTES)
    {
        return Err(format!(
            "its licence notice is longer than {LICENCE_BYTES} bytes"
        ));
    }
    Ok(Offer {
        asset: OfferedAsset {
            environment: reference.environment().to_owned(),
            version: reference.version(),
            name: reference.name().to_owned(),
            digest: reference.digest().to_string(),
            length,
            kind: served.kind(),
            licence: served.licence().map(str::to_owned),
        },
        digest: reference.digest(),
        owner: (environment.id().to_owned(), environment.version()),
    })
}

/// The bytes of `served` from `library`, which checks them against its digest, if they are within its kind's bound and
/// start as its kind's must.
fn read(served: &ServedAsset, library: &Library) -> Result<Kept, String> {
    let bytes = library
        .bytes(served.reference())
        .map_err(|error| error.to_string())?;
    let kind = served.kind();
    if bytes.len() as u64 > kind.most_bytes() {
        return Err(format!(
            "it is {} bytes, more than the {} that {} may be",
            bytes.len(),
            kind.most_bytes(),
            kind.content_type()
        ));
    }
    if !kind.recognises(&bytes) {
        return Err(format!("it is not {}", kind.content_type()));
    }
    Ok(Kept {
        bytes: bytes.into(),
        kind,
    })
}

/// What a request's `Range` field asks of content `length` bytes long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
    /// All of it: no range, or one this server answers whole, such as several ranges or another unit.
    Whole,
    /// The bytes from the first to before the second.
    Part(u64, u64),
    /// A range that starts past its end: 416.
    Unsatisfiable,
}

impl Range {
    /// What `field`, a request's `Range`, asks of content `length` bytes long: one range of bytes, `bytes=START-`,
    /// `bytes=START-END` or `bytes=-SUFFIX`, or the whole.
    pub(crate) fn of(field: Option<&str>, length: u64) -> Self {
        let Some(spec) = field.and_then(|field| field.trim().strip_prefix("bytes=")) else {
            return Self::Whole;
        };
        let Some((first, last)) = spec.trim().split_once('-') else {
            return Self::Whole;
        };
        let number = |text: &str| -> Option<Option<u64>> {
            match text.trim() {
                "" => Some(None),
                digits if digits.bytes().all(|byte| byte.is_ascii_digit()) => {
                    digits.parse().ok().map(Some)
                }
                _ => None,
            }
        };
        let (Some(first), Some(last)) = (number(first), number(last)) else {
            return Self::Whole;
        };
        match (first, last) {
            (Some(first), last) if last.is_none_or(|last| last >= first) => {
                if first >= length {
                    Self::Unsatisfiable
                } else {
                    Self::Part(
                        first,
                        last.map_or(length, |last| last.saturating_add(1).min(length)),
                    )
                }
            }
            (None, Some(suffix)) if suffix > 0 => {
                if length == 0 {
                    Self::Unsatisfiable
                } else {
                    Self::Part(length - suffix.min(length), length)
                }
            }
            (None, Some(_)) => Self::Unsatisfiable,
            _ => Self::Whole,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
