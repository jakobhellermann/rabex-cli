//! Verbs that operate on a whole game (`info`, `ls`, `scenes`, `addressable`).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rabex_env::Environment;
use rabex_env::addressables::AddressablesData;
use rabex_env::addressables::binary_catalog::{ResourceLocation, resource_providers};
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

/// Look up an addressables key in the catalog and print its location(s): the
/// provider, internal id, type, and the bundle the asset lives in.
pub fn addressable(env: &Environment, key: &str) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;
    let build_folder = addressables.build_folder();

    let mut found = false;
    for mut catalog in addressables.catalogs(&env.game_files)? {
        let catalog = catalog.read()?;
        for loc in catalog.locations() {
            if loc.primary_key.as_str() != key {
                continue;
            }
            found = true;
            writeln!(out, "{}", loc.primary_key)?;
            writeln!(out, "  provider: {}", loc.provider_name())?;
            writeln!(
                out,
                "  internal id: {}",
                addressables.evaluate_string(&loc.internal_id)
            )?;
            writeln!(out, "  type: {}", loc.type_.m_ClassName)?;
            if let Some(bundle) = location_bundle(addressables, loc, &build_folder) {
                writeln!(out, "  bundle: {}", bundle.display())?;
            }
        }
    }
    if !found {
        bail!("no addressable with key '{key}'");
    }
    Ok(())
}

/// The bundle (relative to the build folder) an addressable location lives in:
/// itself if it is an `AssetBundle`, else its `AssetBundle` dependency.
fn location_bundle(
    addressables: &AddressablesData,
    loc: &ResourceLocation,
    build_folder: &Path,
) -> Option<PathBuf> {
    let internal_id = if loc.provider_id.as_str() == resource_providers::ASSET_BUNDLE {
        &loc.internal_id
    } else {
        &loc.dependencies
            .iter()
            .find(|dep| dep.provider_id.as_str() == resource_providers::ASSET_BUNDLE)?
            .internal_id
    };
    let path = addressables.evaluate_string(internal_id);
    let path = Path::new(&path);
    Some(path.strip_prefix(build_folder).unwrap_or(path).to_owned())
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
