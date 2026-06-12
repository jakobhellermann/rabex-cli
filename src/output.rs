//! Rendering command results in the format chosen by the global `--format`.
//!
//! Each command builds a typed, [`Serialize`]able value that also knows how to
//! render itself as human-readable text ([`Render`]). [`emit`] picks between the
//! two so the text and JSON outputs share one source of truth and can't drift.

use std::io::Write;

use anyhow::Result;
use serde::Serialize;

use crate::cli::Format;

/// A command result that can be rendered as human text or serialized as JSON.
pub trait Render: Serialize {
    /// Write the human-readable (`--format pretty`) form.
    fn render(&self, out: &mut dyn Write) -> Result<()>;
}

/// Emit `value` in the requested format. JSON is pretty-printed (jq-pipeable),
/// one document per command, with a trailing newline.
pub fn emit<T: Render>(value: &T, format: Format, out: &mut dyn Write) -> Result<()> {
    match format {
        Format::Pretty => value.render(out),
        Format::Json => {
            serde_json::to_writer_pretty(&mut *out, value)?;
            writeln!(out)?;
            Ok(())
        }
    }
}

/// `cat` dumps an already-built JSON value; both formats pretty-print it.
impl Render for serde_json::Value {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        serde_json::to_writer_pretty(&mut *out, self)?;
        writeln!(out)?;
        Ok(())
    }
}
