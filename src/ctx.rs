use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rabex_env::Environment;
use rabex_env::game_files::GameFiles;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::tpk::TpkTypeTreeBlob;
use rabex_env::rabex::typetree::typetree_cache::sync::TypeTreeCache;

use crate::target::Target;

/// Shared state handed to every command: the detected target plus an
/// `Environment` rooted at the game the target belongs to.
pub struct Ctx {
    pub target: Target,
    env: Environment,
    /// The target path made relative to `env.game_files.game_dir`, for
    /// loading file/bundle targets. `None` for a `GameDir` target.
    relative: Option<PathBuf>,
}

impl Ctx {
    pub fn new(path: &Path) -> Result<Ctx> {
        let target = Target::detect(path)?;
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
            Target::SerializedFile(file) | Target::Bundle(file) => {
                let (env, game_dir) = env_for_file(file, tpk)?;
                let relative = file.strip_prefix(&game_dir).unwrap_or(file).to_path_buf();
                Ok(Ctx {
                    target,
                    env,
                    relative: Some(relative),
                })
            }
        }
    }

    /// Load the target as a serialized file (file or bundle). Bails on a game dir.
    pub fn load(&self) -> Result<SerializedFileHandle<'_>> {
        let relative = match &self.relative {
            Some(relative) => relative,
            // `relative` is only `None` for a `GameDir` target.
            None => bail!("expected a file or bundle, not a game directory"),
        };

        match &self.target {
            Target::SerializedFile(_) => self.env.load_serialized(relative),
            Target::Bundle(_) => self.env.load_addressables_bundle_content(relative),
            Target::GameDir(_) => unreachable!("game dir target has no relative path"),
        }
    }
}

/// Climb from the file's directory upward until a unity game dir probes
/// successfully. `ancestors()` includes the starting directory itself, so this
/// also covers the case of the file living directly in a `*_Data` dir.
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

    bail!(
        "no unity game directory found at or above {}",
        start.display()
    )
}
