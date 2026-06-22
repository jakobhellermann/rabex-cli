//! Resolving the target of a command from the CLI flags into an
//! [`Environment`] plus, where relevant, a serialized-file handle or bundle
//! reader.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rabex_env::Environment;
use rabex_env::addressables::AddressablesData;
use rabex_env::addressables::binary_catalog::{ResourceLocation, resource_providers};
use rabex_env::env::Data;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};
use rabex_env::rabex::files::serializedfile::SerializedFile;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::tpk::TpkTypeTreeBlob;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::resolver::game_files::LevelFiles;
use rabex_env::resolver::{EnvResolver, GameFiles};
use rabex_env::unity::types::AssetBundle;

use crate::cli::Context;
use crate::commands::file::FileLocation;
use crate::locate::locate_steam_game;

const SCENE_INSTANCE_CLASS: &str = "UnityEngine.ResourceManagement.ResourceProviders.SceneInstance";

pub fn tpk() -> TypeTreeCache<TpkTypeTreeBlob> {
    TypeTreeCache::new(TpkTypeTreeBlob::embedded())
}

/// The game directory from the context flags, or — when none are given — a
/// unity game discovered at or above the current working directory.
fn game_dir(game: &Context) -> Result<Option<PathBuf>> {
    match (&game.steam_game, &game.game_dir) {
        (Some(name), _) => Ok(Some(locate_steam_game(name)?)),
        (None, Some(dir)) => Ok(Some(dir.clone())),
        (None, None) => Ok(game_from_cwd()),
    }
}

/// A unity game directory at or above the current working directory, so running
/// inside a game's folder needs no `--steam-game`/`--game-dir`.
fn game_from_cwd() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors()
        .find(|dir| GameFiles::probe(dir).is_ok())
        .map(Path::to_path_buf)
}

/// The game [`Environment`], if a context was given.
pub fn game_env(game: &Context) -> Result<Option<Environment>> {
    let Some(dir) = game_dir(game)? else {
        return Ok(None);
    };
    let env = Environment::new_in(&dir, tpk())
        .with_context(|| format!("not a unity game dir: {}", dir.display()))?;
    Ok(Some(env))
}

/// The game [`Environment`]; errors if no context was given.
pub fn require_game_env(game: &Context) -> Result<Environment> {
    game_env(game)?.context(
        "no game: pass --steam-game <name> / --game-dir <dir>, or run inside a game directory",
    )
}

/// Build an `Environment` for a standalone file/bundle and the path relative to
/// its root. Climbs the file's ancestors for a surrounding unity game dir (so
/// externals and addressables resolve), falling back to its own directory.
fn standalone(path: &Path) -> Result<(Environment, PathBuf)> {
    let start = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;

    for dir in start.ancestors() {
        if let Ok(game_files) = GameFiles::probe(dir) {
            let root = game_files.game_dir.clone();
            let relative = path.strip_prefix(&root).unwrap_or(path).to_path_buf();
            return Ok((Environment::new(game_files, tpk()), relative));
        }
    }

    let game_files = GameFiles {
        game_dir: start.to_path_buf(),
        level_files: LevelFiles::Unpacked,
    };
    let relative = path.strip_prefix(start).unwrap_or(path).to_path_buf();
    Ok((Environment::new(game_files, tpk()), relative))
}

/// Open a serialized file: with a game context the path is game-relative,
/// otherwise it is a standalone fs path. Returns the owning `Environment` and
/// the path relative to its root; the caller builds the handle via
/// [`Environment::load_serialized`].
pub fn open_file(game: &Context, path: &Path) -> Result<(Environment, PathBuf)> {
    match game_env(game)? {
        Some(env) => Ok((env, path.to_owned())),
        None => standalone(path),
    }
}

/// Open a bundle reader: with a game context the path is an addressables bundle
/// (relative to the build folder), otherwise a raw fs path. Returns the owning
/// `Environment` too, so a contained file can be loaded into a handle.
pub fn open_bundle(
    game: &Context,
    path: &Path,
) -> Result<(Environment, BundleFileReader<Cursor<Data>>)> {
    match game_env(game)? {
        Some(env) => {
            let reader = env.load_addressables_bundle(path)?;
            Ok((env, reader))
        }
        None => {
            let (env, relative) = standalone(path)?;
            let reader = raw_bundle_reader(&env, &relative)?;
            Ok((env, reader))
        }
    }
}

/// Read a raw bundle at `relative` directly through the resolver (rather than
/// joining the addressables build folder), for standalone bundles.
fn raw_bundle_reader(env: &Environment, relative: &Path) -> Result<BundleFileReader<Cursor<Data>>> {
    let data = env
        .game_files
        .read_path(relative)
        .with_context(|| format!("read bundle {}", relative.display()))?;

    // Bundles often omit their unity version; supply the game's if we have one.
    let mut config = ExtractionConfig::default();
    if let Ok(version) = env.unity_version() {
        config = config.with_fallback_unity_version(version.clone());
    }
    BundleFileReader::from_reader(Cursor::new(data), &config)
        .with_context(|| format!("open bundle {}", relative.display()))
}

/// Load a serialized file out of an open bundle into a handle. `cab` selects a
/// contained file by name; `None` takes the bundle's main serialized file.
pub fn bundle_serialized<'a>(
    env: &'a Environment,
    bundle: &BundleFileReader<Cursor<Data>>,
    cab: Option<&str>,
) -> Result<SerializedFileHandle<'a>> {
    let entry = match cab {
        Some(name) => bundle
            .files()
            .iter()
            .find(|entry| entry.path == name)
            .with_context(|| format!("no file '{name}' in bundle"))?,
        None => bundle
            .main_serializedfile()
            .context("bundle contains no serialized file")?,
    };
    let content = bundle
        .read_at(&entry.path)?
        .context("missing bundle entry")?;
    let mut file = SerializedFile::from_reader(&mut Cursor::new(content.as_slice()))?;
    if let Ok(version) = env.unity_version() {
        file.m_UnityVersion.get_or_insert(version.clone());
    }
    Ok(env.insert_cache(entry.path.clone().into(), file, Data::InMemory(content)))
}

/// Where a scene's data lives.
#[derive(Clone)]
pub enum SceneSource {
    /// A built-in scene: the `levelN` serialized file at this build index.
    Level(usize),
    /// An addressables scene, in this bundle (relative to the build folder).
    Bundle(PathBuf),
}

impl SceneSource {
    /// Short label for listings/completion (`level3`, path/to/foo.bundle`).
    pub fn label(&self) -> String {
        match self {
            SceneSource::Level(index) => format!("level{index}"),
            SceneSource::Bundle(path) => format!("{}", path.display()),
        }
    }
}

pub struct Scene {
    pub name: String,
    pub source: SceneSource,
}

/// Resolve a scene by name to a serialized-file handle and the location by
/// which other files reference it (for the `references` verb).
pub fn open_scene<'a>(
    env: &'a Environment,
    name: &str,
) -> Result<(SerializedFileHandle<'a>, FileLocation)> {
    let scene = scenes(env)?
        .into_iter()
        .find(|scene| scene.name == name)
        .with_context(|| format!("no scene '{name}' in build settings or addressables catalog"))?;

    match scene.source {
        SceneSource::Level(index) => {
            let name = format!("level{index}");
            let handle = env.load_serialized(&name)?;
            Ok((handle, FileLocation::File(name)))
        }
        SceneSource::Bundle(path) => {
            let bundle = env.load_addressables_bundle(&path)?;
            let cab = bundle
                .main_serializedfile()
                .context("bundle contains no serialized file")?
                .path
                .clone();
            let handle = bundle_serialized(env, &bundle, None)?;
            Ok((handle, FileLocation::Bundle { cab }))
        }
    }
}

/// The bundle (relative to the build folder) an addressable location lives in:
/// itself if it is an `AssetBundle`, else its `AssetBundle` dependency.
pub fn location_bundle(
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

/// Resolve an addressables key to the serialized file (the bundle's main CAB)
/// the asset lives in, the location for the `references` verb, and the path id
/// of the asset the key points at (its container entry's `asset`).
///
/// Only `BundledAssetProvider` locations are backed by a CAB we can open; other
/// providers (and keys resolving to several bundled assets) error with a hint.
pub fn open_addressable<'a>(
    env: &'a Environment,
    key: &str,
) -> Result<(SerializedFileHandle<'a>, FileLocation, PathId)> {
    let addressables = env
        .addressables()?
        .context("this game has no addressables")?;
    let build_folder = addressables.build_folder();

    // The bundled locations of this key, each with the container key (its
    // evaluated internal id) we later look the main asset up by.
    let mut bundled = Vec::new();
    let mut other_providers = Vec::new();
    for mut catalog in addressables.catalogs(&env.game_files)? {
        let catalog = catalog.read()?;
        if let Some((_, locs)) = catalog.resources.iter().find(|(k, _)| k.as_str() == key) {
            for loc in locs {
                if loc.provider_id.as_str() == resource_providers::BUNDLED_ASSET {
                    if let Some(bundle) = location_bundle(addressables, loc, &build_folder) {
                        let container_key = addressables.evaluate_string(&loc.internal_id);
                        bundled.push((bundle, container_key));
                    }
                } else {
                    other_providers.push(loc.provider_name().to_owned());
                }
            }
        }
    }

    let (bundle_path, container_key) = match bundled.len() {
        0 if other_providers.is_empty() => bail!("no addressable with key '{key}'"),
        0 => bail!(
            "addressable '{key}' has no BundledAsset location to open (provider(s): {}); \
             see `addressable {key} info`",
            other_providers.join(", ")
        ),
        1 => bundled.into_iter().next().unwrap(),
        n => {
            let bundles: Vec<_> = bundled
                .iter()
                .map(|(b, _)| b.display().to_string())
                .collect();
            bail!(
                "addressable '{key}' resolves to {n} bundled assets ({}); \
                 open the bundle directly with `bundle <path> file <cab>`",
                bundles.join(", ")
            )
        }
    };

    let bundle = env.load_addressables_bundle(&bundle_path)?;
    let cab = bundle
        .main_serializedfile()
        .context("bundle contains no serialized file")?
        .path
        .clone();
    let handle = bundle_serialized(env, &bundle, None)?;

    // The bundle's `AssetBundle` maps each container entry (the addressable's
    // internal id) to the `PPtr` of its main asset.
    let ab = handle
        .objects_of::<AssetBundle>()
        .next()
        .context("bundle's main file has no AssetBundle object")?
        .read()?;
    let asset = ab
        .m_Container
        .get(&container_key)
        .map(|entry| entry.asset.m_PathID)
        .with_context(|| format!("AssetBundle container has no entry '{container_key}'"))?;

    Ok((handle, FileLocation::Bundle { cab }, asset))
}

/// Every addressables key mapped to the distinct asset type names it resolves
/// to (e.g. `AreaAbyss` → {`AtmosCue`, `MusicCue`}). Empty without addressables.
///
/// Unless `include_asset_bundles` is set, keys that resolve only to `AssetBundle`
/// resources (the internal `*.bundle` provider entries, `IAssetBundleResource`)
/// are skipped — they're load-machinery keys, not user-facing assets, and can't
/// be opened as an addressable anyway (use `bundle <path>` for those).
pub fn addressable_keys(
    env: &Environment,
    include_asset_bundles: bool,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Some(addressables) = env.addressables()? {
        for mut catalog in addressables.catalogs(&env.game_files)? {
            let catalog = catalog.read()?;
            for (key, locations) in &catalog.resources {
                if !include_asset_bundles
                    && !locations.is_empty()
                    && locations
                        .iter()
                        .all(|loc| loc.provider_id.as_str() == resource_providers::ASSET_BUNDLE)
                {
                    continue;
                }
                let types = keys.entry(key.to_string()).or_default();
                for loc in locations {
                    types.insert(loc.type_.class_name().to_owned());
                }
            }
        }
    }
    Ok(keys)
}

/// All scenes: built-in scenes (from `BuildSettings`, in build order) followed
/// by addressables scenes (deduped, sorted), each tagged with its source.
pub fn scenes<R: EnvResolver, P: TypeTreeProvider>(env: &Environment<R, P>) -> Result<Vec<Scene>> {
    let mut scenes = Vec::new();
    let mut seen = BTreeSet::new();

    for (index, name) in env.build_settings()?.scene_names().enumerate() {
        seen.insert(name.to_owned());
        scenes.push(Scene {
            name: name.to_owned(),
            source: SceneSource::Level(index),
        });
    }

    let mut addressable = BTreeMap::new();
    if let Some(addressables) = env.addressables()? {
        let build_folder = addressables.build_folder();
        for mut catalog in addressables.catalogs(&env.game_files)? {
            let catalog = catalog.read()?;
            for loc in catalog.locations() {
                if loc.provider_id.as_str() != resource_providers::BUNDLED_ASSET
                    || loc.type_.m_ClassName.as_str() != SCENE_INSTANCE_CLASS
                {
                    continue;
                }
                let Some(name) = Path::new(loc.primary_key.as_str())
                    .file_stem()
                    .and_then(|s| s.to_str())
                else {
                    continue;
                };
                if seen.contains(name) || addressable.contains_key(name) {
                    continue;
                }
                // The scene's bundle is its ASSET_BUNDLE dependency.
                if let Some(dep) = loc
                    .dependencies
                    .iter()
                    .find(|dep| dep.provider_id.as_str() == resource_providers::ASSET_BUNDLE)
                {
                    let path = addressables.evaluate_string(&dep.internal_id);
                    let relative = Path::new(&path)
                        .strip_prefix(&build_folder)
                        .unwrap_or(Path::new(&path));
                    addressable.insert(name.to_owned(), relative.to_owned());
                }
            }
        }
    }
    for (name, bundle) in addressable {
        scenes.push(Scene {
            name,
            source: SceneSource::Bundle(bundle),
        });
    }
    Ok(scenes)
}
