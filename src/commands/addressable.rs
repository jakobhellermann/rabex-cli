//! The `addressable <key>` command: where a key's data lives (`info`), plus
//! `cat` / `file` as sugar over the shared file verbs. Resolving a key to a
//! bundle and path id is [`crate::ctx::open_addressable`].

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rabex_env::Environment;
use rabex_env::addressables::AddressablesData;
use rabex_env::addressables::catalog::{ResourceLocation, resource_providers};
use serde::Serialize;

use crate::cli::{
    AddressableArgs, AddressableInfoArgs, AddressableVerb, FileVerb, Format, ObjectArgs, ObjectVerb,
};
use crate::commands::file::run_verb;
use crate::component_path::ObjectRef;
use crate::ctx;
use crate::output::{Render, emit, style};

/// Run an `addressable <key>` command. `info` (the default) renders the key's
/// catalog locations; `cat` is sugar for descending into the bundle's main CAB
/// and dumping the container's main asset by path id; `file` runs the shared
/// file verbs on the bundle's main serialized file.
pub fn run(env: &Environment, args: AddressableArgs, format: Format) -> Result<()> {
    let verb = args
        .verb
        .unwrap_or(AddressableVerb::Info(AddressableInfoArgs {
            dependencies: false,
        }));
    match verb {
        AddressableVerb::Info(info) => addressable_info(env, &args.key, info.dependencies, format),
        AddressableVerb::Cat => {
            let (handle, location, asset) = ctx::open_addressable(env, &args.key)?;
            let verb = FileVerb::Object(ObjectArgs {
                reference: ObjectRef::PathId(asset),
                verb: Some(ObjectVerb::Cat(Default::default())),
            });
            run_verb(location, &handle, Some(verb), format)
        }
        AddressableVerb::File(file) => {
            let (handle, location, _asset) = ctx::open_addressable(env, &args.key)?;
            run_verb(location, &handle, file.verb, format)
        }
    }
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
    for catalog in addressables.catalogs(&env.game_files)? {
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
