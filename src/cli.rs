use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::ArgValueCandidates;

use crate::component_path::ComponentPath;

/// Inspect Unity serialized files, asset bundles and game directories.
///
/// A game context is set with `--steam-game`/`--game-dir` before the verb, e.g.
/// `rabex --steam-game silksong scenes` or
/// `rabex --steam-game silksong file level0 tree`. File/scene/bundle verbs also
/// work standalone on a filesystem path without a game context.
#[derive(Parser)]
#[command(name = "rabex", version, about, disable_help_subcommand = true)]
pub struct Cli {
    #[command(flatten)]
    pub game: GameArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// The game context, shared by every command (optional for standalone paths).
#[derive(Args, Clone)]
pub struct GameArgs {
    /// Locate a game by steam name or app id.
    #[arg(long, global = true, value_name = "NAME", conflicts_with = "game_dir", add = ArgValueCandidates::new(crate::complete::steam_games))]
    pub steam_game: Option<String>,

    /// Path to a unity game directory (its parent or the `*_Data` dir).
    #[arg(long, global = true, value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    pub game_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show summary info about the game.
    Info,
    /// List the game's serialized files.
    Ls,
    /// List scenes (build settings + addressables).
    Scenes,
    /// Look up an addressables key (its catalog location and bundle).
    #[command(visible_alias = "aa")]
    Addressable(AddressableArgs),
    /// Inspect asset bundles (no path: list all bundles).
    Bundle(BundleArgs),
    /// Inspect a serialized file.
    File(FileArgs),
    /// Inspect a scene by name (resolved via build settings / addressables).
    Scene(SceneArgs),
}

#[derive(Args)]
pub struct AddressableArgs {
    /// Addressables key/address (e.g. `_GameCameras`, `Scenes/Menu_Title`).
    #[arg(value_name = "KEY", add = ArgValueCandidates::new(|| crate::complete::addressable_keys().unwrap_or_default()))]
    pub key: String,

    /// List each dependency bundle (default: just the count).
    #[arg(long)]
    pub dependencies: bool,
}

#[derive(Args)]
pub struct FileArgs {
    /// Serialized file: a game-relative path, or a standalone fs path.
    #[arg(value_name = "PATH", add = ArgValueCandidates::new(|| crate::complete::game_files().unwrap_or_default()))]
    pub path: PathBuf,

    #[command(subcommand)]
    pub verb: FileVerb,
}

#[derive(Args)]
pub struct SceneArgs {
    /// Scene name (e.g. `Greymoor_05`).
    #[arg(value_name = "NAME", add = ArgValueCandidates::new(|| crate::complete::scene_names().unwrap_or_default()))]
    pub name: String,

    #[command(subcommand)]
    pub verb: FileVerb,
}

#[derive(Args)]
pub struct BundleArgs {
    /// Bundle path (game-relative or fs). Omit to list all bundles.
    #[arg(value_name = "PATH", add = ArgValueCandidates::new(|| crate::complete::bundle_files().unwrap_or_default()))]
    pub path: Option<PathBuf>,

    #[command(subcommand)]
    pub verb: Option<BundleVerb>,
}

#[derive(Subcommand)]
pub enum BundleVerb {
    /// List entries: all bundles without a path, else the bundle's files.
    Ls,
    /// Show the bundle's contained files.
    Info,
    /// Inspect a serialized file (CAB) inside the bundle.
    File(BundleFileArgs),
}

#[derive(Args)]
pub struct BundleFileArgs {
    /// Name of the file (CAB) inside the bundle.
    #[arg(value_name = "CAB")]
    pub cab: String,

    #[command(subcommand)]
    pub verb: FileVerb,
}

/// What to do with a selected serialized file (shared by file/scene/bundle).
#[derive(Subcommand)]
pub enum FileVerb {
    /// Show header info (version, types, object/external counts).
    Info,
    /// List the objects (`path_id  ClassId`).
    Ls(LsArgs),
    /// Dump a single object by its path id.
    Obj(ObjArgs),
    /// Dump a component (or GameObject) by hierarchy path.
    Cat(CatArgs),
    /// Print the Transform hierarchy.
    Tree(TreeArgs),
}

#[derive(Args)]
pub struct CatArgs {
    /// Reference like `Root/Child@SpriteRenderer`. Disambiguate equal names
    /// with `:<index>` (`A/B:2@Fsm:1`); escape `/ @ :` with `\`. Without a
    /// `@component`, dumps the GameObject itself.
    #[arg(value_name = "PATH", value_parser = crate::component_path::parse, add = ArgValueCandidates::new(|| crate::complete::component_paths().unwrap_or_default()))]
    pub path: ComponentPath,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    pub format: Format,
}

#[derive(Args)]
pub struct TreeArgs {
    /// Root the tree at this GameObject (hierarchy path, e.g. `Root/Child`);
    /// omit to start from every scene root.
    #[arg(value_name = "PATH", value_parser = crate::component_path::parse, add = ArgValueCandidates::new(|| crate::complete::gameobject_paths().unwrap_or_default()))]
    pub path: Option<ComponentPath>,

    /// Under each GameObject, list its components (class id, or the script name
    /// for MonoBehaviours).
    #[arg(long)]
    pub components: bool,

    /// Like `--components`, but only MonoBehaviours (by script name).
    #[arg(long, conflicts_with = "components")]
    pub scripts: bool,
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
