//! The `files` listing: the game's serialized files.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use rabex_env::Environment;
use rabex_env::resolver::EnvResolver as _;
use serde::Serialize;

use crate::cli::Format;
use crate::output::{Render, emit};

/// The game's serialized files.
#[derive(Serialize)]
#[serde(transparent)]
pub struct Files(pub Vec<PathBuf>);

impl Render for Files {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for path in &self.0 {
            writeln!(out, "{}", path.display())?;
        }
        Ok(())
    }
}

/// List the game's serialized files.
pub fn ls(env: &Environment, format: Format) -> Result<()> {
    let files = Files(env.game_files.serialized_files()?);
    let stdout = std::io::stdout();
    emit(&files, format, &mut stdout.lock())
}
