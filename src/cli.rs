use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::ArgValueCandidates;

use crate::component_path::ObjectRef;

/// Inspect Unity serialized files, asset bundles and game directories.
///
/// A game context is set with `--steam-game`/`--game-dir` before the verb, or
/// detected from the current directory. Plurals list a collection (`scenes`,
/// `bundles`); singulars select one item then operate (`scene <name> tree`,
/// `bundle <path> file <cab> objects`).
#[derive(Parser)]
#[command(name = "rabex", version, about)]
pub struct Cli {
    #[command(flatten)]
    pub game: Context,

    #[command(flatten)]
    pub output: OutputOpts,

    #[command(subcommand)]
    pub command: Command,
}

/// Output options shared by every command.
#[derive(Args, Clone)]
pub struct OutputOpts {
    /// Output format: human-readable text or structured JSON for tooling.
    #[arg(long, global = true, value_enum, default_value_t = Format::Pretty)]
    pub format: Format,

    /// When to colorize `pretty` output.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}

/// When to emit ANSI colors in `pretty` output.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    /// Colorize when stdout is a terminal and `NO_COLOR` is unset (the default).
    Auto,
    /// Always colorize.
    Always,
    /// Never colorize.
    Never,
}

/// The game context, shared by every command (optional for standalone paths,
/// and auto-detected from the current directory).
#[derive(Args, Clone)]
pub struct Context {
    /// Locate a game by steam name or app id.
    #[arg(long, global = true, value_name = "NAME", conflicts_with = "game_dir", add = ArgValueCandidates::new(crate::complete::steam_games))]
    pub steam_game: Option<String>,

    /// Path to a unity game directory (its parent or the `*_Data` dir).
    #[arg(long, global = true, value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    pub game_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Game summary.
    Game(GameArgs),

    /// List scenes (build settings + addressables).
    Scenes(ScenesArgs),
    /// List the game's serialized files.
    Files(FilesArgs),
    /// List asset bundles.
    Bundles(BundlesArgs),
    /// List addressables keys.
    Addressables(AddressablesArgs),

    /// Inspect one scene by name.
    Scene(SceneArgs),
    /// Inspect one serialized file by path.
    File(FileArgs),
    /// Inspect one asset bundle by path.
    Bundle(BundleArgs),
    /// Inspect one addressables key.
    Addressable(AddressableArgs),
}

// -----------------------------------------------------------------------------
// Collections (plural). Bare = list; subcommands leave room for meta verbs.
// -----------------------------------------------------------------------------

#[derive(Args)]
pub struct GameArgs {
    #[command(subcommand)]
    pub verb: Option<GameVerb>,
}
#[derive(Subcommand)]
pub enum GameVerb {
    /// Summary (unity version, file/addressable counts).
    Info,
    /// Each script and the files / addressables that contain its definition.
    ScriptLocations(ScriptLocationsArgs),
}

#[derive(Args)]
pub struct ScriptLocationsArgs {
    /// Only scripts whose full name contains this (case-insensitive).
    #[arg(value_name = "FILTER")]
    pub filter: Option<String>,
}

#[derive(Args)]
pub struct ScenesArgs {
    #[command(subcommand)]
    pub verb: Option<ScenesVerb>,
}
#[derive(Subcommand)]
pub enum ScenesVerb {
    /// List all scenes (the default).
    List,
}

#[derive(Args)]
pub struct FilesArgs {
    #[command(subcommand)]
    pub verb: Option<FilesVerb>,
}
#[derive(Subcommand)]
pub enum FilesVerb {
    /// List the game's serialized files (the default).
    List,
}

#[derive(Args)]
pub struct BundlesArgs {
    #[command(subcommand)]
    pub verb: Option<BundlesVerb>,
}
#[derive(Subcommand)]
pub enum BundlesVerb {
    /// List all bundles (the default).
    List,
}

#[derive(Args)]
pub struct AddressablesArgs {
    #[command(subcommand)]
    pub verb: Option<AddressablesVerb>,
    /// Also list internal `AssetBundle` resource keys (`IAssetBundleResource`),
    /// hidden by default. Inspect those with `bundle <path>` instead.
    #[arg(long)]
    pub include_asset_bundles: bool,
}
#[derive(Subcommand)]
pub enum AddressablesVerb {
    /// List all keys with their asset types (the default).
    List,
    /// Catalog overview: counts and breakdowns by provider/type.
    Stats,
}

// -----------------------------------------------------------------------------
// Items (singular): a selector then a verb.
// -----------------------------------------------------------------------------

#[derive(Args)]
pub struct SceneArgs {
    /// Scene name (e.g. `Greymoor_05`).
    #[arg(value_name = "NAME", add = ArgValueCandidates::new(|| crate::complete::scene_names().unwrap_or_default()))]
    pub name: String,
    #[command(subcommand)]
    pub verb: Option<FileVerb>,
}

#[derive(Args)]
pub struct FileArgs {
    /// Serialized file: a game-relative path, or a standalone fs path.
    #[arg(value_name = "PATH", add = ArgValueCandidates::new(|| crate::complete::game_files().unwrap_or_default()))]
    pub path: PathBuf,
    #[command(subcommand)]
    pub verb: Option<FileVerb>,
}

#[derive(Args)]
pub struct BundleArgs {
    /// Bundle path (game-relative or fs).
    #[arg(value_name = "PATH", add = ArgValueCandidates::new(|| crate::complete::bundle_files().unwrap_or_default()))]
    pub path: PathBuf,
    #[command(subcommand)]
    pub verb: Option<BundleVerb>,
}

#[derive(Subcommand)]
pub enum BundleVerb {
    /// Show the bundle's contained files with sizes.
    Info,
    /// List the bundle's contained files (CABs).
    Files,
    /// Inspect a serialized file (CAB) inside the bundle.
    File(BundleFileArgs),
}

#[derive(Args)]
pub struct BundleFileArgs {
    /// Name of the file (CAB) inside the bundle; defaults to the bundle's main
    /// serialized file.
    #[arg(value_name = "CAB", add = ArgValueCandidates::new(|| crate::complete::bundle_cabs().unwrap_or_default()))]
    pub cab: Option<String>,
    #[command(subcommand)]
    pub verb: Option<FileVerb>,
}

#[derive(Args)]
pub struct AddressableArgs {
    /// Addressables key/address (e.g. `_GameCameras`, `Scenes/Menu_Title`).
    #[arg(value_name = "KEY", add = ArgValueCandidates::new(|| crate::complete::addressable_keys().unwrap_or_default()))]
    pub key: String,
    #[command(subcommand)]
    pub verb: Option<AddressableVerb>,
}

#[derive(Subcommand)]
pub enum AddressableVerb {
    /// Catalog location(s), bundle and dependency count (the default).
    Info(AddressableInfoArgs),
    /// Dump the asset the key points at as JSON (PPtrs annotated with `$ref`).
    Cat,
    /// Inspect the asset's serialized file (the bundle's main CAB) with the
    /// shared file verbs (`tree`, `objects`, `object <ref>`, `preloads`, …).
    File(AddressableFileArgs),
}

#[derive(Args)]
pub struct AddressableInfoArgs {
    /// List each dependency bundle (default: just the count).
    #[arg(long)]
    pub dependencies: bool,
}

#[derive(Args)]
pub struct AddressableFileArgs {
    #[command(subcommand)]
    pub verb: Option<FileVerb>,
}

// -----------------------------------------------------------------------------
// The shared serialized-file verb set, reached via scene / file / bundle-cab.
// -----------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum FileVerb {
    /// Show header info (version, types, object/external counts).
    Info(InfoArgs),
    /// Print the Transform hierarchy.
    Tree(TreeArgs),
    /// List the objects (`path_id  ClassId`).
    Objects(ObjectsArgs),
    /// Inspect one object by path id or component path.
    Object(ObjectArgs),
    /// Show who references this file
    References,
    /// Show the AssetBundle preload table grouped per container entry.
    Preloads(PreloadsArgs),
    /// Find which GameObject(s) carry a component / script of the given type.
    Find(FindArgs),
}

#[derive(Args, Default)]
pub struct InfoArgs {
    /// List the external files (resolved to readable bundle paths); default: just
    /// the count.
    #[arg(long)]
    pub externals: bool,
}

#[derive(Args)]
pub struct FindArgs {
    /// Component class name, or MonoBehaviour script name (e.g. `ToolItemManager`).
    #[arg(value_name = "TYPE", add = ArgValueCandidates::new(|| crate::complete::component_types().unwrap_or_default()))]
    pub r#type: String,
}

#[derive(Args)]
pub struct PreloadsArgs {
    /// Only show container entries whose address contains this substring.
    #[arg(value_name = "ADDRESS")]
    pub address: Option<String>,
    /// Resolve the class of external preloaded objects too (loads dependency bundles; slower).
    #[arg(long)]
    pub resolve: bool,
}

#[derive(Args)]
pub struct TreeArgs {
    /// Root the tree at this GameObject (hierarchy path); else every root.
    #[arg(value_name = "PATH", value_parser = crate::component_path::parse, add = ArgValueCandidates::new(|| crate::complete::gameobject_paths().unwrap_or_default()))]
    pub path: Option<ComponentPath>,
    /// Under each GameObject, list its components.
    #[arg(long)]
    pub components: bool,
    /// Like `--components`, but only MonoBehaviours (by script name).
    #[arg(long, conflicts_with = "components")]
    pub scripts: bool,
}

#[derive(Args)]
pub struct ObjectsArgs {
    /// Only list objects of this class (e.g. `MonoBehaviour`, `Texture2D`).
    #[arg(long)]
    pub r#type: Option<String>,
}

// Path ids are i64 and routinely negative; let clap accept `-8333…` as the
// positional value rather than treating it as an unknown flag.
#[derive(Args)]
#[command(allow_negative_numbers = true)]
pub struct ObjectArgs {
    /// A path id (e.g. `-8333…`), an object's `m_Name` (e.g. a `MonoScript`'s
    /// `PlayMakerFSM`), a class name for a singleton (e.g. `TagManager`), or a
    /// component path (`Root/Child@SpriteRenderer`).
    #[arg(value_name = "REF", value_parser = crate::component_path::parse_object_ref, add = ArgValueCandidates::new(|| crate::complete::object_refs().unwrap_or_default()))]
    pub reference: ObjectRef,
    #[command(subcommand)]
    pub verb: Option<ObjectVerb>,
}

#[derive(Subcommand)]
pub enum ObjectVerb {
    /// Object summary (class, name).
    Info,
    /// Dump the object as JSON (PPtrs annotated with `$ref`).
    Cat,
    /// Find every object that references this one (local or from another file).
    References(ReferencesArgs),
}

#[derive(Args)]
pub struct ReferencesArgs {
    /// Also include preload-table references — `AssetBundle` / `PreloadData`
    /// objects list the target as a load-time dependency, not a true user.
    #[arg(long)]
    pub include_preloads: bool,
}

use crate::component_path::ComponentPath;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Human-readable text columns (the default).
    Pretty,
    /// Structured JSON, one document per command.
    Json,
}
