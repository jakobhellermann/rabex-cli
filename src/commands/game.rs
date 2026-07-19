//! Verbs that operate on a whole game (`info`, `ls`, `scenes`, `addressable`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rabex_env::Environment;
use rabex_env::addressables::AddressablesData;
use rabex_env::addressables::binary_catalog::{ResourceLocation, resource_providers};
use rabex_env::resolver::EnvResolver as _;
use rabex_env::unity::types::MonoScript;
use rabex_env::utils::par_fold_reduce;
use serde::Serialize;

use crate::cli::Format;
use crate::ctx;
use crate::output::{Render, emit, style};

/// Summary of a unity game directory.
#[derive(Serialize)]
pub struct GameInfo {
    path: PathBuf,
    unity_version: String,
    serialized_files: usize,
    addressables: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    addressables_bundles: Option<usize>,
}

impl Render for GameInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(out, "{}", style::header("game directory"))?;
        writeln!(out, "  path: {}", style::name(&self.path.display().to_string()))?;
        writeln!(out, "  unity version: {}", self.unity_version)?;
        writeln!(out, "  serialized files: {}", self.serialized_files)?;
        let addressables = if self.addressables { "yes" } else { "no" };
        writeln!(out, "  addressables: {addressables}")?;
        if let Some(bundles) = self.addressables_bundles {
            writeln!(out, "  addressables bundles: {bundles}")?;
        }
        Ok(())
    }
}

/// Build a summary of a unity game directory. A failure to read the
/// addressables is reported to stderr rather than embedded in the result, so
/// the rest of the summary still prints (and stays valid JSON).
pub fn info(env: &Environment, format: Format) -> Result<()> {
    let unity_version = env
        .unity_version()
        .map_or_else(|e| format!("<unknown: {e}>"), |v| v.to_string());
    let serialized_files = env.game_files.serialized_files()?.len();
    let (addressables, addressables_bundles) = match env.addressables() {
        Ok(Some(_)) => (true, env.addressables_bundles().map(|b| b.len()).ok()),
        Ok(None) => (false, None),
        Err(e) => {
            eprintln!("warning: failed to read addressables: {e}");
            (false, None)
        }
    };

    let info = GameInfo {
        path: env.game_files.game_dir.clone(),
        unity_version,
        serialized_files,
        addressables,
        addressables_bundles,
    };
    let stdout = std::io::stdout();
    emit(&info, format, &mut stdout.lock())
}

/// The game's serialized files.
#[derive(Serialize)]
#[serde(transparent)]
pub struct Files(pub Vec<PathBuf>);

impl Render for Files {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for path in &self.0 {
            writeln!(out, "{}", path.display())?;
        }
        Ok(())
    }
}

/// List the game's serialized files.
pub fn ls(env: &Environment, format: Format) -> Result<()> {
    let files = Files(env.game_files.serialized_files()?);
    let stdout = std::io::stdout();
    emit(&files, format, &mut stdout.lock())
}

/// One script and the files / addressables that contain its definition.
#[derive(Serialize)]
pub struct ScriptLocation {
    script: String,
    locations: Vec<String>,
}

/// Each script (`Namespace.Class`) with the files / addressables it lives in.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ScriptLocations(pub Vec<ScriptLocation>);

impl Render for ScriptLocations {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for entry in &self.0 {
            writeln!(out, "{}", style::class(&entry.script))?;
            for location in &entry.locations {
                writeln!(out, "  {}", style::dim(location))?;
            }
        }
        Ok(())
    }
}

/// A serialized file or an addressables bundle to scan for `MonoScript`s.
enum UnityFile {
    SerializedFile(PathBuf),
    Bundle(PathBuf),
}

/// Map each script to the files / addressables whose `MonoScript` objects
/// define it. Scans every serialized file and addressables bundle in parallel;
/// `filter` keeps only scripts whose full name contains it (case-insensitive).
pub fn script_locations(env: &Environment, filter: Option<&str>, format: Format) -> Result<()> {
    let mut files: Vec<UnityFile> = env
        .game_files
        .serialized_files()?
        .into_iter()
        .map(UnityFile::SerializedFile)
        .collect();
    files.extend(
        env.addressables_bundles()?
            .into_iter()
            .map(UnityFile::Bundle),
    );

    let by_script =
        par_fold_reduce::<BTreeMap<String, BTreeSet<String>>, _>(files, |acc, file| {
            let (handle, location) = match file {
                UnityFile::SerializedFile(path) => {
                    let location = path.display().to_string();
                    (env.load_serialized(&path)?, location)
                }
                UnityFile::Bundle(bundle) => {
                    let location = bundle.display().to_string();
                    (env.load_addressables_bundle_content(&bundle)?, location)
                }
            };
            for script in handle.objects_of::<MonoScript>() {
                let script = script.read()?;
                acc.entry(script.full_name().into_owned())
                    .or_default()
                    .insert(location.clone());
            }
            Ok(())
        })?;

    let filter = filter.map(str::to_ascii_lowercase);
    let locations = by_script
        .into_iter()
        .filter(|(script, _)| match &filter {
            Some(needle) => script.to_ascii_lowercase().contains(needle),
            None => true,
        })
        .map(|(script, locations)| ScriptLocation {
            script,
            locations: locations.into_iter().collect(),
        })
        .collect();

    let stdout = std::io::stdout();
    emit(&ScriptLocations(locations), format, &mut stdout.lock())
}

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

/// One catalog location a key resolves to.
#[derive(Serialize)]
pub struct AddressableLocation {
    #[serde(rename = "type")]
    type_: String,
    primary_key: String,
    internal_id: String,
    provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle: Option<PathBuf>,
    /// Total dependency bundles (`bundle` plus the shared bundles it
    /// transitively references).
    dependencies: usize,
    /// The dependency bundle labels; only populated with `--dependencies`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dependency_labels: Vec<String>,
}

/// The catalog location(s) an addressables key resolves to.
#[derive(Serialize)]
pub struct AddressableInfo {
    key: String,
    locations: Vec<AddressableLocation>,
}

impl Render for AddressableInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        let noun = if self.locations.len() == 1 {
            "location"
        } else {
            "locations"
        };
        writeln!(
            out,
            "{} {}",
            style::name(&self.key),
            style::dim(&format!("({} {noun})", self.locations.len()))
        )?;
        for (i, loc) in self.locations.iter().enumerate() {
            if i > 0 {
                writeln!(out)?;
            }
            writeln!(out, "  {:<14}{}", "type:", style::class(&loc.type_))?;
            writeln!(out, "  {:<14}{}", "primary key:", loc.primary_key)?;
            writeln!(out, "  {:<14}{}", "internal id:", loc.internal_id)?;
            writeln!(out, "  {:<14}{}", "provider:", style::class(&loc.provider))?;
            if let Some(bundle) = &loc.bundle {
                writeln!(out, "  {:<14}{}", "bundle:", bundle.display())?;
            }
            // The full set of bundles needed for this asset (its own bundle plus the
            // shared bundles it transitively references) is often huge — show the
            // count, and only list them with `--dependencies`.
            if loc.dependencies > 0 {
                writeln!(out, "  {:<14}{}", "dependencies:", loc.dependencies)?;
                for label in &loc.dependency_labels {
                    writeln!(out, "    {}", style::dim(label))?;
                }
            }
        }
        Ok(())
    }
}

/// Look up an addressables key in the catalog and build the location(s) it maps
/// to — the same set `Addressables.Load*(key)` would resolve. A key can map to
/// several assets (it may be a label), so each is listed with its type,
/// internal id and bundle.
pub fn addressable_info(
    env: &Environment,
    key: &str,
    list_deps: bool,
    format: Format,
) -> Result<()> {
    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;
    let build_folder = addressables.build_folder();

    // The catalog maps a key to a list of locations; that — not the per-location
    // `primary_key`, which isn't unique — is what the key resolves to.
    let mut raw = Vec::new();
    for mut catalog in addressables.catalogs(&env.game_files)? {
        let catalog = catalog.read()?;
        if let Some((_, locs)) = catalog.resources.iter().find(|(k, _)| k.as_str() == key) {
            raw.extend(locs.iter().cloned());
        }
    }
    if raw.is_empty() {
        bail!("no addressable with key '{key}'");
    }

    let locations = raw
        .iter()
        .map(|loc| AddressableLocation {
            type_: loc.type_.class_name().to_owned(),
            primary_key: loc.primary_key.to_string(),
            internal_id: addressables.evaluate_string(&loc.internal_id),
            provider: loc.provider_name().to_owned(),
            bundle: ctx::location_bundle(addressables, loc, &build_folder),
            dependencies: loc.dependencies.len(),
            dependency_labels: if list_deps {
                loc.dependencies
                    .iter()
                    .map(|dep| dependency_label(addressables, dep, &build_folder))
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect();

    let info = AddressableInfo {
        key: key.to_owned(),
        locations,
    };
    let stdout = std::io::stdout();
    emit(&info, format, &mut stdout.lock())
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

/// One scene with the source its data lives in.
#[derive(Serialize)]
pub struct SceneEntry {
    name: String,
    source: String,
}

/// All scenes (built-in + addressables), each tagged with its source.
#[derive(Serialize)]
#[serde(transparent)]
pub struct Scenes(pub Vec<SceneEntry>);

impl Render for Scenes {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        let width = self.0.iter().map(|s| s.name.len()).max().unwrap_or(0);
        for scene in &self.0 {
            writeln!(
                out,
                "{}  {}",
                style::name(&format!("{:<width$}", scene.name)),
                style::dim(&scene.source)
            )?;
        }
        Ok(())
    }
}

/// List scenes (built-in + addressables), each tagged with its source.
pub fn scenes(env: &Environment, format: Format) -> Result<()> {
    let scenes = ctx::scenes(env)?
        .into_iter()
        .map(|scene| SceneEntry {
            name: scene.name,
            source: scene.source.label(),
        })
        .collect();
    let stdout = std::io::stdout();
    emit(&Scenes(scenes), format, &mut stdout.lock())
}
