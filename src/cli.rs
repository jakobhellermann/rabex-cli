use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::ArgValueCandidates;

/// Inspect Unity serialized files, asset bundles and game directories.
///
/// The target is selected with options *before* the verb, e.g.
/// `rabex --file foo.bundle ls` or `rabex --steam-game silksong info`.
#[derive(Parser)]
#[command(name = "rabex", version, about)]
pub struct Cli {
    #[command(flatten)]
    pub target: TargetArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show summary info about the target.
    Info,
    /// List the objects (or game files) of the target.
    Ls(LsArgs),
    /// Dump a single object by its path id.
    Obj(ObjArgs),
}

/// How the user points rabex at something to inspect.
///
/// A *game context* may be set with `--steam-game` or `--game-dir`. A *file* or
/// *bundle* may be selected with `--file`/`--bundle`, interpreted relative to
/// the game context if one is set, or as a standalone path otherwise. Which
/// combinations make sense is validated at runtime (see `Target::resolve`).
#[derive(Args, Clone)]
pub struct TargetArgs {
    /// Locate a game by steam name or app id (resolves to its `*_Data` dir).
    #[arg(long, value_name = "NAME", conflicts_with = "game_dir", add = ArgValueCandidates::new(|| crate::complete::steam_games()))]
    pub steam_game: Option<String>,

    /// Path to a unity game directory (its parent or the `*_Data` dir).
    #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    pub game_dir: Option<PathBuf>,

    /// A serialized file: relative to the game context, or a standalone path.
    #[arg(long, value_name = "PATH", conflicts_with = "bundle", add = ArgValueCandidates::new(|| crate::complete::game_files().unwrap_or_default()))]
    pub file: Option<PathBuf>,

    /// An addressables bundle (relative to the game), or a standalone bundle path.
    #[arg(long, value_name = "PATH", add = ArgValueCandidates::new(|| crate::complete::bundle_files().unwrap_or_default()))]
    pub bundle: Option<PathBuf>,
}

#[derive(Args)]
pub struct LsArgs {
    /// Only list objects of this class (e.g. `MonoBehaviour`, `Texture2D`).
    #[arg(long)]
    pub r#type: Option<String>,
}

// Path ids are i64 and routinely negative; let clap accept `-8333…` as the
// positional value rather than treating it as an unknown flag.
#[derive(Args)]
#[command(allow_negative_numbers = true)]
pub struct ObjArgs {
    /// The path id of the object to dump.
    #[arg(add = ArgValueCandidates::new(|| crate::complete::path_ids().unwrap_or_default()))]
    pub path_id: i64,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    pub format: Format,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    /// serde_json over the typetree-read value (jq-pipeable).
    Json,
}
