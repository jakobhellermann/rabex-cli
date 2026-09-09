//! The `game` commands: the whole-game summary (`info`) and the script
//! locations listing (`script-locations`). The scan behind `script-locations`
//! lives in [`crate::ctx`].

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use rabex_env::Environment;
use rabex_env::resolver::EnvResolver as _;
use serde::Serialize;

use crate::cli::Format;
use crate::ctx;
use crate::output::{Render, emit, style};

/// Summary of a unity game directory.
#[derive(Serialize)]
pub struct GameInfo {
    path: PathBuf,
    unity_version: String,
    serialized_files: usize,
    addressables: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    addressables_bundles: Option<usize>,
}

impl Render for GameInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        writeln!(out, "{}", style::header("game directory"))?;
        writeln!(
            out,
            "  path: {}",
            style::name(&self.path.display().to_string())
        )?;
        writeln!(out, "  unity version: {}", self.unity_version)?;
        writeln!(out, "  serialized files: {}", self.serialized_files)?;
        let addressables = if self.addressables { "yes" } else { "no" };
        writeln!(out, "  addressables: {addressables}")?;
        if let Some(bundles) = self.addressables_bundles {
            writeln!(out, "  addressables bundles: {bundles}")?;
        }
        Ok(())
    }
}

/// Build a summary of a unity game directory. A failure to read the
/// addressables is reported to stderr rather than embedded in the result, so
/// the rest of the summary still prints (and stays valid JSON).
pub fn info(env: &Environment, format: Format) -> Result<()> {
    let unity_version = env
        .unity_version()
        .map_or_else(|e| format!("<unknown: {e}>"), |v| v.to_string());
    let serialized_files = env.game_files.serialized_files()?.len();
    let (addressables, addressables_bundles) = match env.addressables() {
        Ok(Some(_)) => (true, env.addressables_bundles().map(|b| b.len()).ok()),
        Ok(None) => (false, None),
        Err(e) => {
            eprintln!("warning: failed to read addressables: {e}");
            (false, None)
        }
    };

    let info = GameInfo {
        path: env.game_files.game_dir.clone(),
        unity_version,
        serialized_files,
        addressables,
        addressables_bundles,
    };
    let stdout = std::io::stdout();
    emit(&info, format, &mut stdout.lock())
}

/// One script and the files / addressables that contain its definition.
#[derive(Serialize)]
pub struct ScriptLocation {
    script: String,
    locations: Vec<String>,
}

/// Each script (`Namespace.Class`) with the files / addressables it lives in.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ScriptLocations(pub Vec<ScriptLocation>);

impl Render for ScriptLocations {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for entry in &self.0 {
            writeln!(out, "{}", style::class(&entry.script))?;
            for location in &entry.locations {
                writeln!(out, "  {}", style::dim(location))?;
            }
        }
        Ok(())
    }
}

/// Map each script to the files / addressables whose `MonoScript` objects
/// define it (see [`ctx::script_locations`]). `filter` keeps only scripts
/// whose full name contains it (case-insensitive).
pub fn script_locations(env: &Environment, filter: Option<&str>, format: Format) -> Result<()> {
    let filter = filter.map(str::to_ascii_lowercase);
    let locations = ctx::script_locations(env)?
        .into_iter()
        .filter(|(script, _)| match &filter {
            Some(needle) => script.to_ascii_lowercase().contains(needle),
            None => true,
        })
        .map(|(script, locations)| ScriptLocation {
            script,
            locations: locations.into_iter().collect(),
        })
        .collect();

    let stdout = std::io::stdout();
    emit(&ScriptLocations(locations), format, &mut stdout.lock())
}
