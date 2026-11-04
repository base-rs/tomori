// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! `tomori-server auth mint|list|revoke`: manage the tokens in the server's state directory. A running server applies changes within a few seconds. See `design/security.md`.

use std::path::{Path, PathBuf};

use tomori_security::{Grant, TokenStore};

pub const USAGE: &str =
    "Usage: tomori-server auth mint --owner --name NAME [--state-directory PATH]
       tomori-server auth list [--state-directory PATH]
       tomori-server auth revoke ID [--state-directory PATH]

  mint      Issue an owner token and print it once. Give it to a client with --token, the
            tomori.token intent extra, or the sign-in page.
  list      List the issued tokens by identifier, name and role, without revealing them.
  revoke    Revoke the token with this identifier; its connections end.";

/// The default state directory: `$XDG_STATE_HOME/tomori`, or else `~/.local/state/tomori`.
pub fn default_state_directory(state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match (
        state_home.filter(|path| !path.is_empty()),
        home.filter(|path| !path.is_empty()),
    ) {
        (Some(state_home), _) => Some(Path::new(state_home).join("tomori")),
        (None, Some(home)) => Some(Path::new(home).join(".local/state/tomori")),
        (None, None) => None,
    }
}

/// Run an `auth` command, returning what to print, or an error.
pub fn run(
    mut arguments: impl Iterator<Item = String>,
    default_state: Option<PathBuf>,
) -> Result<String, String> {
    let command = arguments.next().ok_or(USAGE)?;
    let mut state = default_state;
    let mut owner = false;
    let mut name = None;
    let mut id = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--state-directory" => {
                state = Some(
                    arguments
                        .next()
                        .ok_or("--state-directory needs a directory")?
                        .into(),
                )
            }
            "--owner" if command == "mint" => owner = true,
            "--name" if command == "mint" => {
                name = Some(arguments.next().ok_or("--name needs a name")?)
            }
            "--help" | "-h" => return Err(USAGE.to_owned()),
            other if command == "revoke" && id.is_none() && !other.starts_with('-') => {
                id = Some(other.to_owned())
            }
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    let state = state.ok_or("no state directory: set --state-directory, XDG_STATE_HOME or HOME")?;
    let open = || {
        TokenStore::open(&state).map_err(|error| {
            format!(
                "Could not open the token store in {}: {error}.",
                state.display()
            )
        })
    };
    match command.as_str() {
        "mint" => {
            if !owner {
                return Err(format!("mint needs --owner\n\n{USAGE}"));
            }
            let name = name
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| format!("mint needs --name\n\n{USAGE}"))?;
            let (entry, token) = open()?
                .issue(name.trim(), Grant::Owner)
                .map_err(|error| format!("Could not issue the token: {error}."))?;
            Ok(format!(
                "Issued owner token {} for {:?}. It is shown only once:\n\n{token}\n",
                entry.id, entry.name
            ))
        }
        "list" => {
            let entries = open()?.entries();
            if entries.is_empty() {
                return Ok("No tokens are issued.\n".to_owned());
            }
            Ok(entries
                .iter()
                .map(|entry| {
                    let role = match &entry.grant {
                        Grant::Owner => "owner".to_owned(),
                        Grant::Collaborator { space } => format!("collaborator in {space}"),
                    };
                    format!("{}  {}  {:?}\n", entry.id, role, entry.name)
                })
                .collect())
        }
        "revoke" => {
            let id = id.ok_or_else(|| format!("revoke needs a token identifier\n\n{USAGE}"))?;
            if open()?
                .revoke(&id)
                .map_err(|error| format!("Could not revoke the token: {error}."))?
            {
                Ok(format!("Revoked token {id}.\n"))
            } else {
                Err(format!("No token has the identifier {id:?}."))
            }
        }
        other => Err(format!("unknown auth command {other:?}\n\n{USAGE}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_in(state: &Path, arguments: &[&str]) -> Result<String, String> {
        run(
            arguments.iter().map(|argument| (*argument).to_owned()),
            Some(state.to_owned()),
        )
    }

    #[test]
    fn the_default_state_directory_follows_xdg() {
        assert_eq!(
            default_state_directory(Some("/state"), Some("/home/ada")),
            Some(PathBuf::from("/state/tomori"))
        );
        assert_eq!(
            default_state_directory(Some(""), Some("/home/ada")),
            Some(PathBuf::from("/home/ada/.local/state/tomori"))
        );
        assert_eq!(default_state_directory(None, None), None);
    }

    #[test]
    fn mints_lists_and_revokes_owner_tokens() {
        let state =
            std::env::temp_dir().join(format!("tomori-auth-command-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        assert_eq!(
            run_in(&state, &["list"]).unwrap(),
            "No tokens are issued.\n"
        );

        let minted = run_in(&state, &["mint", "--owner", "--name", " Ada's Quest "]).unwrap();
        let token = minted.lines().last().unwrap();
        assert!(token.starts_with(tomori_security::TOKEN_PREFIX), "{minted}");
        let store = TokenStore::open(&state).unwrap();
        let entry = store.entries().remove(0);
        assert_eq!(entry.name, "Ada's Quest");

        let listed = run_in(&state, &["list"]).unwrap();
        assert_eq!(listed, format!("{}  owner  \"Ada's Quest\"\n", entry.id));
        assert!(!listed.contains(token));
        store
            .issue(
                "Bob",
                Grant::Collaborator {
                    space: "studio".into(),
                },
            )
            .unwrap();
        assert!(
            run_in(&state, &["list"])
                .unwrap()
                .contains("collaborator in studio  \"Bob\"")
        );

        assert_eq!(
            run_in(&state, &["revoke", &entry.id]).unwrap(),
            format!("Revoked token {}.\n", entry.id)
        );
        assert!(run_in(&state, &["revoke", &entry.id]).is_err());
        std::fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn refuses_incomplete_or_unknown_commands() {
        let state = std::env::temp_dir().join(format!("tomori-auth-refuse-{}", std::process::id()));
        for arguments in [
            &[][..],
            &["mint", "--name", "Ada"],
            &["mint", "--owner"],
            &["mint", "--owner", "--name", " "],
            &["mint", "--owner", "--name"],
            &["revoke"],
            &["list", "--owner"],
            &["forge"],
            &["list", "--help"],
            &["list", "--state-directory"],
        ] {
            assert!(run_in(&state, arguments).is_err(), "{arguments:?}");
        }
        assert!(run(["list".to_owned()].into_iter(), None).is_err());
        let elsewhere = state.join("elsewhere");
        let listed = run(
            ["list", "--state-directory", elsewhere.to_str().unwrap()]
                .iter()
                .map(|argument| (*argument).to_owned()),
            None,
        )
        .unwrap();
        assert_eq!(listed, "No tokens are issued.\n");
        let _ = std::fs::remove_dir_all(state);
    }
}
