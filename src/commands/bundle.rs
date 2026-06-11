//! The `bundle` command: list all bundles, or inspect one bundle's contents
//! and drill into a contained serialized file.

use std::io::{Cursor, Write};

use anyhow::{Result, bail};
use rabex_env::env::Data;
use rabex_env::rabex::files::bundlefile::BundleFileReader;
use rabex_env::rabex::files::unityfile::FileEntry;

use crate::cli::{BundleArgs, BundleVerb, GameArgs};
use crate::commands::file;
use crate::ctx;

pub fn run(game: &GameArgs, args: BundleArgs) -> Result<()> {
    match args.path {
        // No path: list all bundles in the game.
        None => match args.verb {
            None | Some(BundleVerb::Ls) => list_all(game),
            Some(_) => bail!("no bundle path given (use `bundle <path> …`)"),
        },
        Some(path) => {
            let (env, bundle) = ctx::open_bundle(game, &path)?;
            match args.verb {
                None | Some(BundleVerb::Ls) => list_files(&bundle),
                Some(BundleVerb::Info) => info(&bundle),
                Some(BundleVerb::File(file_args)) => {
                    let handle = ctx::bundle_serialized(&env, &bundle, Some(&file_args.cab))?;
                    file::run(&handle, file_args.verb)
                }
            }
        }
    }
}

/// List every addressables bundle in the game.
fn list_all(game: &GameArgs) -> Result<()> {
    let env = ctx::require_game_env(game)?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for bundle in env.addressables_bundles()? {
        writeln!(out, "{}", bundle.display())?;
    }
    Ok(())
}

/// List the files contained in a bundle.
fn list_files(bundle: &BundleFileReader<Cursor<Data>>) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for entry in bundle.files() {
        writeln!(out, "{}", entry.path)?;
    }
    Ok(())
}

/// Show the bundle's contained files with sizes and kinds.
fn info(bundle: &BundleFileReader<Cursor<Data>>) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let entries = bundle.files();
    writeln!(out, "bundle ({} files)", entries.len())?;
    for entry in entries {
        let kind = if entry.flags & FileEntry::FLAG_SERIALIZEDFILE != 0 {
            "serialized"
        } else {
            "resource"
        };
        writeln!(out, "  {:>10}  {:<11}  {}", entry.size, kind, entry.path)?;
    }
    Ok(())
}
