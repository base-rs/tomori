// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! `tomori-server auth mint|list|rename|revoke`: manage the tokens in the server's state directory. A running server applies changes within a few seconds. See `design/security.md`.
use std::path::{Path, PathBuf};

use tomori_security::{Grant, TokenStore};

pub const USAGE: &str =
    "Usage: tomori-server auth mint --owner --name NAME [--state-directory PATH]
       tomori-server auth list [--state-directory PATH]
       tomori-server auth rename ID NAME [--state-directory PATH]
       tomori-server auth revoke ID [--state-directory PATH]

  mint      Issue an owner token and print it once. Give it to a client with --token, the
            tomori.token intent extra, or the sign-in page.
  list      List the issued tokens by identifier, role, name and tag, without revealing
            them.
  rename    Change the display name of the token with this identifier, which everyone
            using it is known by, in spaces and to the agent.
  revoke    Revoke the token with this identifier; its connections end.";

/// The default state directory: `$XDG_STATE_HOME/tomori`, or else `~/.local/state/tomori`. A relative `XDG_STATE_HOME` is ignored, as the XDG Base Directory specification requires, so that the token store never lands relative to the working directory.
pub fn default_state_directory(state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match (
        state_home.filter(|path| Path::new(path).is_absolute()),
        home.filter(|path| !path.is_empty()),
    ) {
        (Some(state_home), _) => Some(Path::new(state_home).join("tomori")),
        (None, Some(home)) => Some(Path::new(home).join(".local/state/tomori")),
        (None, None) => None,
    }
}

/// [`USAGE`], for the program named `program`, such as `tomori-server`.
pub(crate) fn usage(program: &str) -> String {
    USAGE.replace("tomori-server", program)
}

/// Run an `auth` command, returning what to print, or an error.
pub fn run(
    arguments: impl Iterator<Item = String>,
    default_state: Option<PathBuf>,
) -> Result<String, String> {
    run_named("tomori-server", arguments, default_state)
}

/// Run an `auth` command of the program named `program`, as [`run`] does, its usage naming `program`.
pub(crate) fn run_named(
    program: &str,
    mut arguments: impl Iterator<Item = String>,
    default_state: Option<PathBuf>,
) -> Result<String, String> {
    let usage = usage(program);
    let Some(command) = arguments.next() else {
        return Err(usage);
    };
    let mut state = default_state;
    let mut owner = false;
    let mut name = None;
    let mut id = None;
    let mut new_name = None;
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
            "--help" | "-h" => return Err(usage),
            other
                if matches!(command.as_str(), "revoke" | "rename")
                    && id.is_none()
                    && !other.starts_with('-') =>
            {
                id = Some(other.to_owned())
            }
            other if command == "rename" && id.is_some() && new_name.is_none() => {
                new_name = Some(other.to_owned())
            }
            other => return Err(format!("unknown argument {other:?}\n\n{usage}")),
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
                return Err(format!("mint needs --owner\n\n{usage}"));
            }
            let name = name.ok_or_else(|| format!("mint needs --name\n\n{usage}"))?;
            let name = crate::limits::name(&name)?;
            let (entry, token) = open()?
                .issue(&name, Grant::Owner)
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
                    // Its holder's handle, as the agent names them where others share the name.
                    let tag = entry
                        .tag
                        .map_or_else(String::new, |tag| format!("  #{tag}"));
                    format!("{}  {}  {:?}{tag}\n", entry.id, role, entry.name)
                })
                .collect())
        }
        "rename" => {
            let (Some(id), Some(name)) = (id, new_name) else {
                return Err(format!(
                    "rename needs a token identifier and a name\n\n{usage}"
                ));
            };
            let name = crate::limits::name(&name)?;
            // Another token's name is no bar: tags tell their holders apart.
            match open()?
                .rename(&id, &name, |_| false)
                .map_err(|error| format!("Could not rename the token: {error}."))?
            {
                tomori_security::Renamed::Renamed => {
                    Ok(format!("Renamed token {id} to {name:?}.\n"))
                }
                _ => Err(format!("No token has the identifier {id:?}.")),
            }
        }
        "revoke" => {
            let id = id.ok_or_else(|| format!("revoke needs a token identifier\n\n{usage}"))?;
            if open()?
                .revoke(&id)
                .map_err(|error| format!("Could not revoke the token: {error}."))?
            {
                Ok(format!("Revoked token {id}.\n"))
            } else {
                Err(format!("No token has the identifier {id:?}."))
            }
        }
        other => Err(format!("unknown auth command {other:?}\n\n{usage}")),
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
    fn a_relative_xdg_state_home_is_ignored() {
        for relative in ["state", "./state", "../state", "."] {
            assert_eq!(
                default_state_directory(Some(relative), Some("/home/ada")),
                Some(PathBuf::from("/home/ada/.local/state/tomori")),
                "{relative:?}"
            );
            assert_eq!(default_state_directory(Some(relative), None), None);
        }
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
        assert_eq!(
            listed,
            format!("{}  owner  \"Ada's Quest\"  #10\n", entry.id)
        );
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

        // Renaming keeps the token, and its holders are known by the new name.
        assert_eq!(
            run_in(&state, &["rename", &entry.id, " Ada Lovelace "]).unwrap(),
            format!("Renamed token {} to \"Ada Lovelace\".\n", entry.id)
        );
        assert!(
            run_in(&state, &["list"])
                .unwrap()
                .starts_with(&format!("{}  owner  \"Ada Lovelace\"  #10\n", entry.id))
        );
        assert_eq!(
            run_in(&state, &["rename", &entry.id, "/admin"]).unwrap_err(),
            "A name must not start with \"/\"."
        );
        assert_eq!(
            run_in(&state, &["rename", "missing", "Ada"]).unwrap_err(),
            "No token has the identifier \"missing\"."
        );
        // A name is cleaned before it is kept; one another token has is theirs to take too, since tags tell them apart.
        assert_eq!(
            run_in(&state, &["rename", &entry.id, "Ada\u{202E}\n Lovelace"]).unwrap(),
            format!("Renamed token {} to \"Ada Lovelace\".\n", entry.id)
        );
        assert_eq!(
            run_in(&state, &["rename", &entry.id, "Ada: hi"]).unwrap_err(),
            "A name must not contain \":\", \"[\" or \"]\"."
        );
        assert_eq!(
            run_in(&state, &["rename", &entry.id, "b\u{200B}OB"]).unwrap(),
            format!("Renamed token {} to \"bOB\".\n", entry.id)
        );
        assert!(
            run_in(&state, &["list"])
                .unwrap()
                .contains("collaborator in studio  \"Bob\"  #11")
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
            &["mint", "--owner", "--name", "/admin"],
            &["revoke"],
            &["rename"],
            &["rename", "abc"],
            &["rename", "abc", "Ada", "extra"],
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

    #[test]
    fn the_usage_names_the_program() {
        let help = |program: &str, arguments: &[&str]| {
            run_named(
                program,
                arguments.iter().map(|argument| (*argument).to_owned()),
                // Never opened, since none of these commands is run.
                Some(std::env::temp_dir()),
            )
            .unwrap_err()
        };
        assert_eq!(help("tomori-server", &[]), USAGE);
        assert_eq!(
            run(["list", "-h"].map(str::to_owned).into_iter(), None).unwrap_err(),
            USAGE
        );
        // Another program's, every line of it.
        let usage = help("observe-server", &["list", "--help"]);
        assert!(usage.starts_with(
            "Usage: observe-server auth mint --owner --name NAME [--state-directory PATH]\n       observe-server auth list"
        ));
        assert!(usage.contains("\n       observe-server auth revoke ID"));
        assert!(!usage.contains("tomori-server"));
        assert_eq!(
            help("observe-server", &["forge"]),
            format!("unknown auth command \"forge\"\n\n{usage}")
        );
    }

    #[test]
    fn a_store_that_cannot_be_opened_or_written_is_said_so() {
        // A file where the state directory should be.
        let state =
            std::env::temp_dir().join(format!("tomori-auth-unusable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        let _ = std::fs::remove_file(&state);
        std::fs::write(&state, "not a directory").unwrap();
        for arguments in [
            &["mint", "--owner", "--name", "Ada"][..],
            &["list"][..],
            &["rename", "abc", "Ada"][..],
            &["revoke", "abc"][..],
        ] {
            let error = run_in(&state, arguments).unwrap_err();
            assert!(
                error.starts_with("Could not open the token store"),
                "{error}"
            );
        }
        std::fs::remove_file(&state).unwrap();
        // A store that cannot be changed, since its lock file cannot be opened where a directory has its name.
        std::fs::create_dir_all(state.join("tokens.lock")).unwrap();
        let error = run_in(&state, &["mint", "--owner", "--name", "Ada"]).unwrap_err();
        assert!(error.starts_with("Could not issue the token"), "{error}");
        let error = run_in(&state, &["revoke", "abc"]).unwrap_err();
        assert!(error.starts_with("Could not revoke the token"), "{error}");
        let error = run_in(&state, &["rename", "abc", "Ada"]).unwrap_err();
        assert!(error.starts_with("Could not rename the token"), "{error}");
        std::fs::remove_dir_all(&state).unwrap();
    }
}
