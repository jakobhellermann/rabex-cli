//! Rendering command results in the format chosen by the global `--format`.
//!
//! Each command builds a typed, [`Serialize`]able value that also knows how to
//! render itself as human-readable text ([`Render`]). [`emit`] picks between the
//! two so the text and JSON outputs share one source of truth and can't drift.

use std::io::Write;
use std::sync::OnceLock;

use anyhow::Result;
use serde::Serialize;

use crate::cli::Format;

static COLOR: OnceLock<bool> = OnceLock::new();

/// Enable/disable ANSI coloring of `pretty` output. Set once at startup from the
/// `--color` choice (tty + `NO_COLOR` aware); left unset — and thus disabled —
/// when rendering outside the binary, so tests stay plain and byte-identical.
pub fn set_color(enabled: bool) {
    let _ = COLOR.set(enabled);
}

fn color_enabled() -> bool {
    COLOR.get().copied().unwrap_or(false)
}

/// Tasteful ANSI styling for `pretty` output. Every helper is a no-op — returns
/// its input unchanged — when color is disabled, so non-tty / `NO_COLOR` /
/// `--color never` output stays plain. Pad to a column width *before* styling, as
/// the ANSI escapes would otherwise be counted by `{:<width$}`.
pub mod style {
    use super::color_enabled;

    fn paint(code: &str, s: &str) -> String {
        if color_enabled() {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    }

    /// Section titles and summary / count lines.
    pub fn header(s: &str) -> String {
        paint("1", s)
    }
    /// The `key:` labels in key/value blocks, path ids, and other secondary text.
    pub fn dim(s: &str) -> String {
        paint("2", s)
    }
    /// Class ids, type names, MonoBehaviour script class names.
    pub fn class(s: &str) -> String {
        paint("36", s)
    }
    /// Object / GameObject / scene names and addressable keys.
    pub fn name(s: &str) -> String {
        paint("32", s)
    }
}

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
