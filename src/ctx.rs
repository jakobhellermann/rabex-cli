use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rabex_env::Environment;
use rabex_env::env::Data;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::files::bundlefile::{BundleFileReader, ExtractionConfig};
use rabex_env::rabex::files::serializedfile::SerializedFile;
use rabex_env::rabex::tpk::TpkTypeTreeBlob;
use rabex_env::rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::resolver::game_files::LevelFiles;
use rabex_env::resolver::{EnvResolver as _, GameFiles};

use crate::cli::TargetArgs;
use crate::target::Target;

/// Shared state handed to every command: the resolved target plus an
/// `Environment` rooted at the relevant game (or, for a standalone target, at
/// the file's own directory).
pub struct Ctx {
    pub target: Target,
    env: Environment,
    /// Path of the file/bundle relative to the env root. `None` for a `GameDir`
    /// target.
    relative: Option<PathBuf>,
}

impl Ctx {
    pub fn new(args: &TargetArgs) -> Result<Ctx> {
        let target = Target::resolve(args)?;
        let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());

        match &target {
            Target::GameDir(dir) => {
                let env = Environment::new_in(dir, tpk)
                    .with_context(|| format!("not a unity game dir: {}", dir.display()))?;
                Ok(Ctx {
                    target,
                    env,
                    relative: None,
                })
            }
            Target::SerializedFile { game_dir, path } | Target::Bundle { game_dir, path } => {
                let (env, relative) = match game_dir {
                    // File/bundle is relative to an explicit game directory.
                    Some(dir) => {
                        let env = Environment::new_in(dir, tpk)
                            .with_context(|| format!("not a unity game dir: {}", dir.display()))?;
                        (env, path.clone())
                    }
                    // Standalone: root the env at (or above) the file itself.
                    None => {
                        let (env, root) = env_for_file(path, tpk)?;
                        let relative = path.strip_prefix(&root).unwrap_or(path).to_path_buf();
                        (env, relative)
                    }
                };
                Ok(Ctx {
                    target,
                    env,
                    relative: Some(relative),
                })
            }
        }
    }

    pub fn env(&self) -> &Environment {
        &self.env
    }

    /// Load the target as a serialized file (file or bundle). Bails on a game dir.
    pub fn load(&self) -> Result<SerializedFileHandle<'_>> {
        let relative = match &self.relative {
            Some(relative) => relative,
            // `relative` is only `None` for a `GameDir` target.
            None => bail!("expected a file or bundle, not a game directory"),
        };

        match &self.target {
            Target::SerializedFile { .. } => self.env.load_serialized(relative),
            // With a game context a `--bundle` path is an addressables bundle
            // (relative to the build folder); standalone it's a raw path.
            Target::Bundle {
                game_dir: Some(_), ..
            } => self.env.load_addressables_bundle_content(relative),
            Target::Bundle { game_dir: None, .. } => self.load_raw_bundle(relative),
            Target::GameDir(_) => unreachable!("game dir target has no relative path"),
        }
    }

    /// Open the bundle target's `BundleFileReader` (e.g. to list its entries).
    /// Bails unless the target is a bundle.
    pub fn open_bundle(&self) -> Result<BundleFileReader<Cursor<Data>>> {
        match &self.target {
            Target::Bundle { game_dir, .. } => {
                let relative = self.relative.as_ref().expect("bundle target has a path");
                match game_dir {
                    Some(_) => self.env.load_addressables_bundle(relative),
                    None => self.raw_bundle_reader(relative),
                }
            }
            _ => bail!("expected a bundle"),
        }
    }

    /// Open a standalone bundle at a raw path through the env resolver.
    ///
    /// `Environment::load_addressables_bundle` joins the addressables build
    /// folder onto the path; this reads the file directly instead, so a bundle
    /// outside any game (`--bundle some.bundle` with no game context) works.
    fn raw_bundle_reader(&self, relative: &Path) -> Result<BundleFileReader<Cursor<Data>>> {
        let data = self
            .env
            .game_files
            .read_path(relative)
            .with_context(|| format!("read bundle {}", relative.display()))?;

        // Bundles often omit their unity version; supply the game's if we have
        // one (a standalone bundle without a game around it just goes without).
        let mut config = ExtractionConfig::default();
        if let Ok(version) = self.env.unity_version() {
            config = config.with_fallback_unity_version(version.clone());
        }
        BundleFileReader::from_reader(Cursor::new(data), &config)
            .with_context(|| format!("open bundle {}", relative.display()))
    }

    /// Load the main serialized file out of a standalone (raw-path) bundle.
    fn load_raw_bundle(&self, relative: &Path) -> Result<SerializedFileHandle<'_>> {
        let bundle = self.raw_bundle_reader(relative)?;

        let entry = bundle
            .main_serializedfile()
            .context("bundle contains no serialized file")?;
        let content = bundle
            .read_at(&entry.path)?
            .context("missing bundle entry")?;
        let mut file = SerializedFile::from_reader(&mut Cursor::new(content.as_slice()))?;
        if let Ok(version) = self.env.unity_version() {
            file.m_UnityVersion.get_or_insert(version.clone());
        }

        Ok(self
            .env
            .insert_cache(entry.path.clone().into(), file, Data::InMemory(content)))
    }
}

/// Build an `Environment` for a standalone file or bundle, returning it
/// together with the directory paths are resolved against.
///
/// Climbs from the file's directory upward looking for a real unity game dir
/// (`*_Data`); rooting there means externals and addressables resolve. If none
/// is found, falls back to a bare `GameFiles` rooted at the file's own
/// directory — enough to load and inspect the file itself, but with no game
/// context (addressables/external lookups simply come up empty).
fn env_for_file(
    file: &Path,
    tpk: TypeTreeCache<TpkTypeTreeBlob>,
) -> Result<(Environment, PathBuf)> {
    let start = file
        .parent()
        .with_context(|| format!("{} has no parent directory", file.display()))?;

    for dir in start.ancestors() {
        if let Ok(game_files) = GameFiles::probe(dir) {
            let game_dir = game_files.game_dir.clone();
            return Ok((Environment::new(game_files, tpk), game_dir));
        }
    }

    let game_files = GameFiles {
        game_dir: start.to_path_buf(),
        level_files: LevelFiles::Unpacked,
    };
    Ok((Environment::new(game_files, tpk), start.to_path_buf()))
}
