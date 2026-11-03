// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Admission at the door: the credentials on the HTTP request that opens a connection decide what it is admitted as, before it reaches the hub. See `design/security.md`.

use std::sync::Arc;

use security::{Authenticator, Chain, Credentials, Grant as Granted, Outcome, TokenStore, Verdict};

use crate::access::{Access, Grant};

/// The server's authentication mode.
pub enum Door {
    /// `--auth open`: everyone is an owner, and credentials are ignored.
    Open,
    /// Tokens the server issued admit their holders; others are visitors, if visitors are allowed.
    Tokens {
        chain: Chain,
        store: Arc<TokenStore>,
        visitors: bool,
    },
}

/// The token store as an authenticator in the chain.
struct Tokens(Arc<TokenStore>);

impl Authenticator for Tokens {
    fn authenticate(&self, credentials: &Credentials) -> Verdict {
        self.0.authenticate(credentials)
    }
}

impl Door {
    /// Admit holders of the tokens in `store`, and, if `visitors`, anyone else to watch.
    pub fn tokens(store: Arc<TokenStore>, visitors: bool) -> Self {
        Self::Tokens {
            chain: Chain::new().with(Tokens(store.clone())),
            store,
            visitors,
        }
    }

    /// What `credentials` admit, or `None` if they are refused: credentials that no authenticator accepts, or none where visitors are not allowed.
    pub fn admit(&self, credentials: &Credentials) -> Option<Access> {
        let Self::Tokens {
            chain,
            store,
            visitors,
        } = self
        else {
            return Some(Access::open());
        };
        match chain.authenticate(credentials) {
            Outcome::Anonymous => visitors.then(Access::visitor),
            Outcome::Refused => None,
            Outcome::Authenticated(principal) => {
                // The store decides what the principal may do; a token revoked since it authenticated admits nothing.
                let grant = match store.entry(&principal.id)?.grant {
                    Granted::Owner => Grant::Owner,
                    Granted::Collaborator { space } => Grant::Collaborator { space },
                };
                Some(Access {
                    principal: Some(principal.id),
                    grant,
                })
            }
        }
    }

    /// The token store, unless everyone is admitted.
    pub fn store(&self) -> Option<&Arc<TokenStore>> {
        match self {
            Self::Open => None,
            Self::Tokens { store, .. } => Some(store),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Arc<TokenStore>, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("tomori-door-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        (Arc::new(TokenStore::open(&directory).unwrap()), directory)
    }

    fn token(token: &str) -> Credentials {
        Credentials {
            token: Some(token.to_owned()),
        }
    }

    #[test]
    fn open_admits_everyone_as_an_owner() {
        assert_eq!(Door::Open.admit(&token("anything")), Some(Access::open()));
        assert!(Door::Open.store().is_none());
    }

    #[test]
    fn tokens_admit_their_grant_and_others_are_visitors_or_refused() {
        let (store, directory) = store("grants");
        let (owner, owner_token) = store.issue("Ada", Granted::Owner).unwrap();
        let (_, collaborator_token) = store
            .issue(
                "Bob",
                Granted::Collaborator {
                    space: "studio".into(),
                },
            )
            .unwrap();
        let door = Door::tokens(store.clone(), true);
        assert_eq!(
            door.admit(&token(&owner_token)),
            Some(Access {
                principal: Some(owner.principal().id),
                grant: Grant::Owner,
            })
        );
        assert!(matches!(
            door.admit(&token(&collaborator_token)),
            Some(Access { grant: Grant::Collaborator { space }, .. }) if space == "studio"
        ));
        assert_eq!(door.admit(&Credentials::default()), Some(Access::visitor()));
        assert_eq!(door.admit(&token("unknown")), None);
        assert_eq!(door.admit(&token(&security::generate_token())), None);
        assert!(door.store().is_some());

        let closed = Door::tokens(store, false);
        assert_eq!(closed.admit(&Credentials::default()), None);
        assert!(closed.admit(&token(&owner_token)).is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
