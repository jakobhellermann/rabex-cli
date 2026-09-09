//! The `addressables` commands: the key listing (`list`) and the catalog
//! overview (`stats`). The key data comes from [`crate::ctx::addressable_keys`].

use std::io::Write;

use anyhow::{Context as _, Result};
use rabex_env::Environment;
use rabex_env::addressables::catalog::ResourceLocation;
use serde::Serialize;

use crate::cli::Format;
use crate::ctx;
use crate::output::{Render, emit, style};

/// Catalog overview: counts plus a breakdown of locations by provider and type.
#[derive(Serialize)]
pub struct AddressableStats {
    catalogs: usize,
    keys: usize,
    locations: usize,
    location_refs: usize,
    bundles: usize,
    /// `(name, count)`, sorted by count descending then name.
    by_provider: Vec<(String, usize)>,
    by_type: Vec<(String, usize)>,
}

impl Render for AddressableStats {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(out, "{}", style::header("addressables"))?;
        writeln!(out, "  catalogs:  {}", self.catalogs)?;
        writeln!(out, "  keys:      {}", self.keys)?;
        writeln!(
            out,
            "  locations: {} ({} refs)",
            self.locations, self.location_refs
        )?;
        writeln!(out, "  bundles:   {}", self.bundles)?;
        render_breakdown(out, "by provider", &self.by_provider)?;
        render_breakdown(out, "by type", &self.by_type)?;
        Ok(())
    }
}

/// Build the catalog overview: counts and a breakdown of locations.
pub fn addressable_stats(env: &Environment, format: Format) -> Result<()> {
    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;

    let mut catalogs = 0usize;
    let mut keys = 0usize;
    let mut refs = 0usize;
    let mut seen = std::collections::HashSet::new();
    let mut by_provider: std::collections::HashMap<String, usize> = Default::default();
    let mut by_type: std::collections::HashMap<String, usize> = Default::default();

    for catalog in addressables.catalogs(&env.game_files)? {
        catalogs += 1;
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

    let stats = AddressableStats {
        catalogs,
        keys,
        locations: seen.len(),
        location_refs: refs,
        bundles: addressables.bundle_paths().count(),
        by_provider: sorted_breakdown(by_provider),
        by_type: sorted_breakdown(by_type),
    };
    let stdout = std::io::stdout();
    emit(&stats, format, &mut stdout.lock())
}

/// Sort a `name → count` map by count descending, then name ascending.
fn sorted_breakdown(counts: std::collections::HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
}

/// Print a `name → count` breakdown under a heading.
fn render_breakdown(out: &mut dyn Write, heading: &str, entries: &[(String, usize)]) -> Result<()> {
    writeln!(out)?;
    writeln!(out, "  {heading}:")?;
    for (name, count) in entries {
        writeln!(
            out,
            "    {}  {}",
            style::dim(&format!("{count:>6}")),
            style::class(name)
        )?;
    }
    Ok(())
}

/// One addressables key with the asset type(s) it resolves to.
#[derive(Serialize)]
pub struct AddressableKey {
    key: String,
    types: Vec<String>,
}

/// Every addressables key with the asset type(s) it resolves to.
#[derive(Serialize)]
#[serde(transparent)]
pub struct AddressableKeys(pub Vec<AddressableKey>);

impl Render for AddressableKeys {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for entry in &self.0 {
            writeln!(
                out,
                "{}  {}",
                style::name(&entry.key),
                style::class(&format!("({})", entry.types.join(", ")))
            )?;
        }
        Ok(())
    }
}

/// List every addressables key with the asset type(s) it resolves to.
pub fn addressable_ls(
    env: &Environment,
    include_asset_bundles: bool,
    format: Format,
) -> Result<()> {
    let keys = ctx::addressable_keys(env, include_asset_bundles)?
        .into_iter()
        .map(|(key, types)| AddressableKey {
            key,
            types: types.into_iter().collect(),
        })
        .collect();
    let stdout = std::io::stdout();
    emit(&AddressableKeys(keys), format, &mut stdout.lock())
}
