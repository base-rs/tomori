// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The client half of Tomori's GitHub environment prototype: a client presents it over the grid, at the identity in `tomori-environment-github`, and the shared scene draws its cards, which the server sends a space that shows them. It never holds the server's GitHub token. See `design/github.md`.
pub use tomori_environment_github::{ID, LABEL, VERSION};

/// Present the GitHub environment over the grid, at [`ID`] and [`VERSION`], which a server registering it with `tomori-environment-github-server` offers it at, so that a client lists it in its `Hello` and shows a space in it without a notice.
///
/// ```
/// let mut sceneries = tomori_render::scenery::Sceneries::empty();
/// tomori_environment_github_client::register(&mut sceneries);
/// let presented = sceneries.over_grid();
/// assert_eq!((presented[0].id(), presented[0].version()), ("github", 1));
/// ```
pub fn register(sceneries: &mut tomori_render::scenery::Sceneries) {
    sceneries.present_over_grid(ID, VERSION);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_github_environment_is_presented_over_the_grid_at_its_identity() {
        let mut sceneries = tomori_render::scenery::Sceneries::empty();
        super::register(&mut sceneries);
        let presented: Vec<(&str, u32)> = sceneries
            .over_grid()
            .into_iter()
            .map(|environment| (environment.id(), environment.version()))
            .collect();
        assert_eq!(presented, [(super::ID, super::VERSION)]);
        // It has no scenery of its own: the grid is drawn, with the scene's cards.
        assert_eq!(sceneries.environments().len(), 1);
    }
}
