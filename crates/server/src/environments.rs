// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environments a space can be bound to. Each space is bound to exactly one, chosen when it is created; see `design/environments.md`.

use protocol::EnvironmentInfo;

/// The moonlit sky, rendered by `crates/environment-sky`. The identifier is the one clients map to that renderer.
pub const SKY: &str = "sky";

/// The environments offered to clients in `Welcome`. Today that is only Sky.
pub fn offered() -> Vec<EnvironmentInfo> {
    vec![EnvironmentInfo {
        id: SKY.to_owned(),
        label: "Sky".to_owned(),
    }]
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
        assert_eq!(offered.len(), 1);
        assert_eq!(choose(&offered, Some(SKY)), Ok(SKY.to_owned()));
        assert_eq!(choose(&offered, None), Ok(SKY.to_owned()));
        let refused = choose(&offered, Some("observe")).unwrap_err();
        assert_eq!(refused, "Unknown environment; available: sky.");
        // Minimal is the arrival grid, not an environment a space is bound to.
        assert!(choose(&offered, Some("minimal")).is_err());
        assert_eq!(
            choose(&[], None),
            Err("The server offers no environments.".to_owned())
        );
    }
}
