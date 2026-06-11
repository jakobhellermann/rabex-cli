//! Verbs that operate on a whole game (`info`, `ls`, `scenes`).

use std::io::Write;

use anyhow::Result;
use rabex_env::Environment;
use rabex_env::resolver::EnvResolver as _;

use crate::ctx;

/// Summary of a unity game directory.
pub fn info(env: &Environment) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let unity_version = env
        .unity_version()
        .map_or_else(|e| format!("<unknown: {e}>"), |v| v.to_string());
    let serialized = env.game_files.serialized_files()?.len();
    let (addressables, bundles) = match env.addressables() {
        Ok(Some(_)) => ("yes", env.addressables_bundles().map(|b| b.len()).ok()),
        Ok(None) => ("no", None),
        Err(_) => ("error", None),
    };

    writeln!(out, "game directory")?;
    writeln!(out, "  unity version: {unity_version}")?;
    writeln!(out, "  serialized files: {serialized}")?;
    writeln!(out, "  addressables: {addressables}")?;
    if let Some(bundles) = bundles {
        writeln!(out, "  addressables bundles: {bundles}")?;
    }
    Ok(())
}

/// List the game's serialized files.
pub fn ls(env: &Environment) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for path in env.game_files.serialized_files()? {
        writeln!(out, "{}", path.display())?;
    }
    Ok(())
}

/// List scenes (built-in + addressables), each tagged with its source.
pub fn scenes(env: &Environment) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let scenes = ctx::scenes(env)?;
    let width = scenes.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for scene in scenes {
        writeln!(out, "{:<width$}  {}", scene.name, scene.source.label())?;
    }
    Ok(())
}
