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

/// Catalog overview: counts plus a breakdown of locations by provider and type.
pub fn addressable_stats(env: &Environment) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;

    let mut catalogs = 0usize;
    let mut keys = 0usize;
    let mut refs = 0usize;
    let mut seen = std::collections::HashSet::new();
    let mut by_provider: std::collections::HashMap<String, usize> = Default::default();
    let mut by_type: std::collections::HashMap<String, usize> = Default::default();

    for mut catalog in addressables.catalogs(&env.game_files)? {
        catalogs += 1;
        let catalog = catalog.read()?;
        keys += catalog.resources.len();
        for loc in catalog.locations() {
            refs += 1;
            if seen.insert(loc as *const ResourceLocation) {
                *by_provider
                    .entry(loc.provider_name().to_owned())
                    .or_default() += 1;
                *by_type
                    .entry(loc.type_.class_name().to_owned())
                    .or_default() += 1;
            }
        }
    }

    writeln!(out, "addressables")?;
    writeln!(out, "  catalogs:  {catalogs}")?;
    writeln!(out, "  keys:      {keys}")?;
    writeln!(out, "  locations: {} ({refs} refs)", seen.len())?;
    writeln!(out, "  bundles:   {}", addressables.bundle_paths().count())?;
    write_breakdown(&mut out, "by provider", by_provider)?;
    write_breakdown(&mut out, "by type", by_type)?;
    Ok(())
}

/// Print a `name → count` map under a heading, sorted by count descending.
fn write_breakdown(
    out: &mut impl Write,
    heading: &str,
    counts: std::collections::HashMap<String, usize>,
) -> Result<()> {
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    writeln!(out)?;
    writeln!(out, "  {heading}:")?;
    for (name, count) in entries {
        writeln!(out, "    {count:>6}  {name}")?;
    }
    Ok(())
}

/// List every addressables key with the asset type(s) it resolves to.
pub fn addressable_ls(env: &Environment) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for (key, types) in ctx::addressable_keys(env)? {
        let types = types.into_iter().collect::<Vec<_>>().join(", ");
        writeln!(out, "{key}  ({types})")?;
    }
    Ok(())
}

/// Look up an addressables key in the catalog and print the location(s) it maps
/// to — the same set `Addressables.Load*(key)` would resolve. A key can map to
/// several assets (it may be a label), so each is listed with its type,
/// internal id and bundle.
pub fn addressable_info(env: &Environment, key: &str, list_deps: bool) -> Result<()> {
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
