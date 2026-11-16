//! The `addressables` commands: the key listing (`list`), the catalog
//! overview (`stats`) and the raw catalog contents (`catalog`). The key data
//! comes from [`crate::ctx::addressable_keys`].

use std::io::Write;
use std::path::Path;

use anyhow::{Context as _, Result};
use rabex_env::Environment;
use rabex_env::addressables::catalog::{self, AddressablesCatalog, ObjectInitializationData};
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
    let mut locations = 0usize;
    let mut by_provider: std::collections::HashMap<String, usize> = Default::default();
    let mut by_type: std::collections::HashMap<String, usize> = Default::default();

    for catalog in addressables.catalogs(&env.game_files)? {
        catalogs += 1;
        keys += catalog.resources.len();
        refs += catalog.resources.values().map(Vec::len).sum::<usize>();
        locations += catalog.locations.len();
        for loc in catalog.locations() {
            *by_provider
                .entry(loc.provider_name().to_owned())
                .or_default() += 1;
            *by_type
                .entry(loc.type_.class_name().to_owned())
                .or_default() += 1;
        }
    }

    let stats = AddressableStats {
        catalogs,
        keys,
        locations,
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

/// A catalog's providers, locations and keys.
#[derive(Serialize)]
pub struct CatalogDump {
    locator_id: String,
    build_result_hash: Option<String>,
    instance_provider: String,
    scene_provider: String,
    resource_providers: Vec<String>,
    locations: Vec<CatalogLocation>,
    keys: Vec<CatalogKeyEntry>,
}

#[derive(Serialize)]
struct CatalogLocation {
    index: usize,
    provider: String,
    #[serde(rename = "type")]
    type_: String,
    primary_key: String,
    internal_id: String,
    bundle_name: Option<String>,
    bundle_size: Option<u32>,
    dependencies: Vec<usize>,
}

#[derive(Serialize)]
struct CatalogKeyEntry {
    key: String,
    locations: Vec<usize>,
}

impl CatalogDump {
    fn new(catalog: &AddressablesCatalog) -> Self {
        let provider = |data: &ObjectInitializationData| data.id.to_string();
        CatalogDump {
            locator_id: catalog.locator_id.to_string(),
            build_result_hash: catalog
                .build_result_hash
                .as_ref()
                .map(|hash| hash.to_string()),
            instance_provider: provider(&catalog.instance_provider_data),
            scene_provider: provider(&catalog.scene_provider_data),
            resource_providers: catalog
                .resource_provider_data
                .iter()
                .map(provider)
                .collect(),
            locations: catalog
                .locations()
                .enumerate()
                .map(|(index, loc)| CatalogLocation {
                    index,
                    provider: loc.provider_name().to_owned(),
                    type_: loc.type_.class_name().to_owned(),
                    primary_key: loc.primary_key.to_string(),
                    internal_id: loc.internal_id.to_string(),
                    bundle_name: loc.abro().map(|abro| abro.bundle_name.to_string()),
                    bundle_size: loc.abro().map(|abro| abro.bundle_size),
                    dependencies: loc.dependencies.clone(),
                })
                .collect(),
            keys: catalog
                .resources
                .iter()
                .map(|(key, locations)| CatalogKeyEntry {
                    key: key.as_str().to_owned(),
                    locations: locations.clone(),
                })
                .collect(),
        }
    }
}

impl Render for CatalogDump {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(out, "{}", style::header(&self.locator_id))?;
        if let Some(hash) = &self.build_result_hash {
            writeln!(out, "  build hash: {hash}")?;
        }
        writeln!(
            out,
            "  instance:   {}",
            style::class(&self.instance_provider)
        )?;
        writeln!(out, "  scene:      {}", style::class(&self.scene_provider))?;
        writeln!(out, "  providers:")?;
        for provider in &self.resource_providers {
            writeln!(out, "    {}", style::class(provider))?;
        }

        writeln!(out)?;
        writeln!(out, "  locations ({}):", self.locations.len())?;
        for loc in &self.locations {
            write!(
                out,
                "    {} {} {} {}",
                style::dim(&format!("#{:<5}", loc.index)),
                style::class(&format!("{}<{}>", loc.provider, loc.type_)),
                style::name(&loc.primary_key),
                loc.internal_id,
            )?;
            if let (Some(name), Some(size)) = (&loc.bundle_name, loc.bundle_size) {
                write!(
                    out,
                    " {}",
                    style::dim(&format!("bundle={name} size={size}"))
                )?;
            }
            if !loc.dependencies.is_empty() {
                write!(
                    out,
                    " {}",
                    style::dim(&format!("deps={}", loc.dependencies.len()))
                )?;
            }
            writeln!(out)?;
        }

        writeln!(out)?;
        writeln!(out, "  keys ({}):", self.keys.len())?;
        for key in &self.keys {
            let locations: Vec<String> = key.locations.iter().map(|i| format!("#{i}")).collect();
            writeln!(
                out,
                "    {} {}",
                style::name(&key.key),
                style::dim(&locations.join(" "))
            )?;
        }
        Ok(())
    }
}

/// Dump a catalog file.
pub fn catalog_file(path: &Path, format: Format) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let catalog = catalog::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    let stdout = std::io::stdout();
    emit(&CatalogDump::new(&catalog), format, &mut stdout.lock())
}

/// Dump every catalog of the game.
pub fn game_catalogs(env: &Environment, format: Format) -> Result<()> {
    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;
    let dumps: Vec<CatalogDump> = addressables
        .catalogs(&env.game_files)?
        .iter()
        .map(CatalogDump::new)
        .collect();
    let stdout = std::io::stdout();
    emit(&CatalogDumps(dumps), format, &mut stdout.lock())
}

#[derive(Serialize)]
#[serde(transparent)]
pub struct CatalogDumps(Vec<CatalogDump>);

impl Render for CatalogDumps {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for (i, dump) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(out)?;
            }
            dump.render(out)?;
        }
        Ok(())
    }
}
