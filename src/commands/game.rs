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

/// Look up an addressables key in the catalog and print the location(s) it maps
/// to — the same set `Addressables.Load*(key)` would resolve. A key can map to
/// several assets (it may be a label), so each is listed with its type,
/// internal id and bundle.
pub fn addressable(env: &Environment, key: &str, list_deps: bool) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;
    let build_folder = addressables.build_folder();

    // The catalog maps a key to a list of locations; that — not the per-location
    // `primary_key`, which isn't unique — is what the key resolves to.
    let mut locations = Vec::new();
    for mut catalog in addressables.catalogs(&env.game_files)? {
        let catalog = catalog.read()?;
        if let Some((_, locs)) = catalog.resources.iter().find(|(k, _)| k.as_str() == key) {
            locations.extend(locs.iter().cloned());
        }
    }
    if locations.is_empty() {
        bail!("no addressable with key '{key}'");
    }

    let noun = if locations.len() == 1 {
        "location"
    } else {
        "locations"
    };
    writeln!(out, "{key} ({} {noun})", locations.len())?;
    for (i, loc) in locations.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        writeln!(out, "  {:<14}{}", "type:", loc.type_.class_name())?;
        writeln!(out, "  {:<14}{}", "primary key:", loc.primary_key)?;
        writeln!(
            out,
            "  {:<14}{}",
            "internal id:",
            addressables.evaluate_string(&loc.internal_id)
        )?;
        writeln!(out, "  {:<14}{}", "provider:", loc.provider_name())?;
        if let Some(bundle) = location_bundle(addressables, loc, &build_folder) {
            writeln!(out, "  {:<14}{}", "bundle:", bundle.display())?;
        }
        // The full set of bundles needed for this asset (its own bundle plus the
        // shared bundles it transitively references) is often huge — show the
        // count, and only list them with `--dependencies`.
        if !loc.dependencies.is_empty() {
            writeln!(out, "  {:<14}{}", "dependencies:", loc.dependencies.len())?;
            if list_deps {
                for dep in &loc.dependencies {
                    writeln!(
                        out,
                        "    {}",
                        dependency_label(addressables, dep, &build_folder)
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// A dependency's display label: its bundle path (relative to the build folder)
/// if it is an `AssetBundle`, else its evaluated internal id plus provider.
fn dependency_label(
    addressables: &AddressablesData,
    dep: &ResourceLocation,
    build_folder: &Path,
) -> String {
    let id = addressables.evaluate_string(&dep.internal_id);
    if dep.provider_id.as_str() == resource_providers::ASSET_BUNDLE {
        let path = Path::new(&id);
        path.strip_prefix(build_folder)
            .unwrap_or(path)
            .display()
            .to_string()
    } else {
        format!("{id}  ({})", dep.provider_name())
    }
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
