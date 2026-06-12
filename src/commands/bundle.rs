//! The `bundle` command: inspect one bundle's contents and drill into a
//! contained serialized file. (`bundles` lists all bundles.)

use std::io::{Cursor, Write};

use anyhow::Result;
use rabex_env::Environment;
use rabex_env::env::Data;
use rabex_env::rabex::files::bundlefile::BundleFileReader;
use rabex_env::rabex::files::unityfile::FileEntry;

use crate::cli::{BundleArgs, BundleVerb, Context};
use crate::commands::file::{self, FileLocation};
use crate::ctx;

pub fn run(game: &Context, args: BundleArgs) -> Result<()> {
    let (env, bundle) = ctx::open_bundle(game, &args.path)?;
    match args.verb.unwrap_or(BundleVerb::Info) {
        BundleVerb::Info => info(&bundle),
        BundleVerb::Files => list_files(&bundle),
        BundleVerb::File(file_args) => {
            let handle = ctx::bundle_serialized(&env, &bundle, Some(&file_args.cab))?;
            let source = FileLocation::Bundle { cab: file_args.cab };
            file::run_verb(source, &handle, file_args.verb)
        }
    }
}

/// List every addressables bundle in the game (`bundles`).
pub fn list_all(env: &Environment) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for bundle in env.addressables_bundles()? {
        writeln!(out, "{}", bundle.display())?;
    }
    Ok(())
}

/// List the files (CABs) contained in a bundle.
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
