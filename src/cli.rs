use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Inspect Unity serialized files, asset bundles and game directories.
#[derive(Parser)]
#[command(name = "rabex", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show summary info about a file, bundle or game directory (auto-detected).
    Info(InfoArgs),
    /// List the objects (or entries) contained in a file or bundle.
    Ls(LsArgs),
    /// Dump a single object by its path id.
    Obj(ObjArgs),
}

impl Command {
    /// The target path, common to every subcommand.
    pub fn path(&self) -> &Path {
        match self {
            Command::Info(a) => &a.path,
            Command::Ls(a) => &a.path,
            Command::Obj(a) => &a.path,
        }
    }
}

#[derive(Args)]
pub struct InfoArgs {
    /// Path to a serialized file, asset bundle, or game directory.
    #[arg(value_hint = clap::ValueHint::AnyPath)]
    pub path: PathBuf,
}

#[derive(Args)]
pub struct LsArgs {
    /// Path to a serialized file or asset bundle.
    #[arg(value_hint = clap::ValueHint::AnyPath)]
    pub path: PathBuf,

    /// Only list objects of this class (e.g. `MonoBehaviour`, `Texture2D`).
    //
    // TODO: dynamic completion from the class ids actually present in `path`.
    #[arg(long)]
    pub r#type: Option<String>,
}

#[derive(Args)]
pub struct ObjArgs {
    /// Path to a serialized file or asset bundle.
    #[arg(value_hint = clap::ValueHint::AnyPath)]
    pub path: PathBuf,

    /// The path id of the object to dump.
    //
    // TODO: dynamic completion from the path ids actually present in `path`.
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
