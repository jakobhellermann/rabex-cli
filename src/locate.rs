//! Steam game location, adapted from unity-scene-repacker.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rabex_env::resolver::GameFiles;

fn search_transform(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Locate a steam-installed unity game by name (fuzzy) or numeric app id,
/// returning its install directory.
pub fn locate_steam_game(game: &str) -> Result<PathBuf> {
    let steam = steamlocate::SteamDir::locate()?;
    let needle = search_transform(game);

    let (app, library) = if let Ok(app_id) = needle.parse() {
        steam
            .find_app(app_id)?
            .with_context(|| format!("no steam game with app id {app_id}"))?
    } else {
        steam
            .libraries()?
            .filter_map(Result::ok)
            .find_map(|library| {
                let mut candidates = library
                    .apps()
                    .filter_map(Result::ok)
                    .filter_map(|app| {
                        let name = app.name.as_ref().unwrap_or(&app.install_dir);
                        let name = search_transform(name);
                        // Prefer the closest match by trailing length.
                        name.contains(&needle)
                            .then(|| (app, name.len() - needle.len()))
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by_key(|&(_, score)| score);
                candidates.into_iter().next().map(|(app, _)| (app, library))
            })
            .with_context(|| format!("no steam game matching '{game}'"))?
    };

    let install_dir = library.resolve_app_dir(&app);
    let name = app.name.as_ref().unwrap_or(&app.install_dir);
    GameFiles::probe_dir(&install_dir)
        .with_context(|| format!("'{name}' is not a unity game at {}", install_dir.display()))?;
    Ok(install_dir)
}

#[cfg(test)]
mod tests {
    use super::search_transform;

    #[test]
    fn ignores_punctuation_and_case() {
        assert_eq!(
            search_transform("Hollow Knight: Silksong"),
            "hollowknightsilksong"
        );
        assert_eq!(
            search_transform("Hollow Knight Silksong"),
            "hollowknightsilksong"
        );
    }
}
