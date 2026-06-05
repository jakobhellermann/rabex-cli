use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cli::TargetArgs;
use crate::locate::locate_steam_game;

/// What the user asked to inspect, resolved from [`TargetArgs`].
#[derive(Debug)]
pub enum Target {
    /// A whole game directory (no file/bundle selected).
    GameDir(PathBuf),
    /// A serialized file, with an optional surrounding game directory.
    SerializedFile {
        game_dir: Option<PathBuf>,
        path: PathBuf,
    },
    /// An asset bundle, with an optional surrounding game directory.
    Bundle {
        game_dir: Option<PathBuf>,
        path: PathBuf,
    },
}

impl Target {
    /// Resolve the target-selection flags into a concrete target.
    ///
    /// The `--steam-game`/`--game-dir` and `--file`/`--bundle` exclusivity is
    /// enforced by clap (`conflicts_with`), so those pairs can't both be set
    /// here.
    pub fn resolve(args: &TargetArgs) -> Result<Target> {
        // The game context, if any.
        let game_dir = match (&args.steam_game, &args.game_dir) {
            (Some(name), _) => Some(locate_steam_game(name)?),
            (None, Some(dir)) => Some(dir.clone()),
            (None, None) => None,
        };

        match (&args.file, &args.bundle) {
            (Some(file), _) => Ok(Target::SerializedFile {
                game_dir,
                path: file.clone(),
            }),
            (None, Some(bundle)) => Ok(Target::Bundle {
                game_dir,
                path: bundle.clone(),
            }),
            (None, None) => match game_dir {
                Some(dir) => Ok(Target::GameDir(dir)),
                None => {
                    bail!("no target given; use --steam-game/--game-dir, and/or --file/--bundle")
                }
            },
        }
    }
}
