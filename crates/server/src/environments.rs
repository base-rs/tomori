// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environments a space can be bound to. Each space is bound to exactly one, chosen when it is created; see `design/environments.md`.

use tomori_protocol::EnvironmentInfo;

/// The moonlit sky, rendered by `crates/environment-sky`. The identifier is the one clients map to that renderer.
pub const SKY: &str = "sky";

/// The plain floor grid, which clients also show outside any space. The identifier is the one clients map to that scenery.
pub const MINIMAL: &str = "minimal";

/// The environments offered to clients in `Welcome`: Sky, first, so that a space whose creator names none is bound to it, and the Minimal grid.
pub fn offered() -> Vec<EnvironmentInfo> {
    [(SKY, "Sky"), (MINIMAL, "Grid")]
        .into_iter()
        .map(|(id, label)| EnvironmentInfo {
            id: id.to_owned(),
            label: label.to_owned(),
        })
        .collect()
}

/// What the environment `id` tells the agent about what it presents and how to use it, its layer of a space's instructions; see `design/spaces.md`. Every environment, Sky included, contributes none until environments can.
pub fn instructions(_id: &str) -> &'static str {
    ""
}

/// The environment to bind a new space to: `requested`, when the server offers it, or the first offered when the request names none, as from a client that predates environments. Refusals name what is offered and never repeat the request.
pub fn choose(offered: &[EnvironmentInfo], requested: Option<&str>) -> Result<String, String> {
    let found = match requested {
        Some(requested) => offered
            .iter()
            .find(|environment| environment.id == requested),
        None => offered.first(),
    };
    match found {
        Some(environment) => Ok(environment.id.clone()),
        None if offered.is_empty() => Err("The server offers no environments.".to_owned()),
        None => {
            let ids: Vec<&str> = offered
                .iter()
                .map(|environment| environment.id.as_str())
                .collect();
            Err(format!(
                "Unknown environment; available: {}.",
                ids.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_offered_environments_are_bound() {
        let offered = offered();
        assert_eq!(offered.len(), 2);
        assert_eq!(choose(&offered, Some(SKY)), Ok(SKY.to_owned()));
        assert_eq!(choose(&offered, Some(MINIMAL)), Ok(MINIMAL.to_owned()));
        // A space whose creator names none is bound to Sky.
        assert_eq!(choose(&offered, None), Ok(SKY.to_owned()));
        let refused = choose(&offered, Some("observe")).unwrap_err();
        assert_eq!(refused, "Unknown environment; available: sky, minimal.");
        assert!(choose(&offered, Some("Minimal")).is_err());
        assert_eq!(
            choose(&[], None),
            Err("The server offers no environments.".to_owned())
        );
    }
}
