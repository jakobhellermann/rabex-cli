//! The `scenes` listing: built-in + addressables scenes, each tagged with its
//! source. The data comes from [`crate::ctx::scenes`].

use std::io::Write;

use anyhow::Result;
use rabex_env::Environment;
use serde::Serialize;
use unicode_width::UnicodeWidthStr as _;

use crate::cli::Format;
use crate::ctx;
use crate::output::{Render, emit, style};

/// One scene with the source its data lives in.
#[derive(Serialize)]
pub struct SceneEntry {
    name: String,
    source: String,
}

/// All scenes (built-in + addressables), each tagged with its source.
#[derive(Serialize)]
#[serde(transparent)]
pub struct Scenes(pub Vec<SceneEntry>);

impl Render for Scenes {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        // Pad by terminal display width, not `char`/byte count, so CJK (and other
        // wide) scene names still line up.
        let width = self.0.iter().map(|s| s.name.width()).max().unwrap_or(0);
        for scene in &self.0 {
            let padding = " ".repeat(width - scene.name.width());
            writeln!(
                out,
                "{}{padding}  {}",
                style::name(&scene.name),
                style::dim(&scene.source)
            )?;
        }
        Ok(())
    }
}

/// List scenes (built-in + addressables), each tagged with its source.
pub fn scenes(env: &Environment, format: Format) -> Result<()> {
    let scenes = ctx::scenes(env)?
        .into_iter()
        .map(|scene| SceneEntry {
            name: scene.name,
            source: scene.source.label(),
        })
        .collect();
    let stdout = std::io::stdout();
    emit(&Scenes(scenes), format, &mut stdout.lock())
}
