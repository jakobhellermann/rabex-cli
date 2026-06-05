//! Dynamic shell completion helpers.
//!
//! `clap_complete` doesn't hand parsed arguments to a completer, so — like jj
//! does — we re-parse `std::env::args_os()` ourselves to recover the `<path>`
//! the user already typed, load it, and offer its path ids as candidates.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::CommandFactory as _;
use clap_complete::CompletionCandidate;

use crate::cli::Cli;
use crate::ctx::Ctx;

/// Recover the `<path>` argument of the subcommand currently being completed.
///
/// We read it straight out of `ArgMatches` rather than going through
/// `Cli::from_arg_matches`, because the latter also parses the (in-progress)
/// `path_id` into an `i64` — which fails on an empty or partial value and would
/// abort completion exactly when the user is at the `path_id` slot.
fn current_path() -> Result<PathBuf> {
    // The clap_complete prelude is `<bin> -- <bin> <actual args...>`, so skip 2.
    let args = std::env::args_os().skip(2);

    let matches = Cli::command()
        .disable_version_flag(true)
        .disable_help_flag(true)
        .ignore_errors(true)
        .try_get_matches_from(args)?;

    let (_subcommand, sub_matches) = matches.subcommand().context("no subcommand")?;
    let path = sub_matches
        .get_one::<PathBuf>("path")
        .context("no `path` argument yet")?;
    Ok(path.clone())
}

/// Candidates for an object path id: every path id present in the target file,
/// labelled with its class id. Clap filters these against the typed prefix.
///
/// Returns a `Result`; the caller decides how to map failures (e.g. to an empty
/// candidate list).
pub fn path_ids() -> Result<Vec<CompletionCandidate>> {
    let path = current_path()?;
    let ctx = Ctx::new(&path)?;
    let file = ctx.load()?;

    let candidates = file
        .objects::<()>()
        .map(|obj| {
            CompletionCandidate::new(obj.path_id().to_string())
                .help(Some(format!("{:?}", obj.class_id()).into()))
        })
        .collect();
    Ok(candidates)
}
