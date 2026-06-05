use std::io::Write;

use anyhow::Result;
use rabex_env::rabex::files::unityfile::FileEntry;
use rabex_env::resolver::EnvResolver as _;

use crate::ctx::Ctx;
use crate::target::Target;

pub fn run(ctx: &Ctx) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match &ctx.target {
        Target::SerializedFile { .. } => serialized_file_info(ctx, &mut out),
        Target::Bundle { .. } => bundle_info(ctx, &mut out),
        Target::GameDir(_) => game_info(ctx, &mut out),
    }
}

/// Header information + type count for a standalone serialized file.
fn serialized_file_info(ctx: &Ctx, out: &mut impl Write) -> Result<()> {
    let handle = ctx.load()?;
    let file = handle.file;
    let header = &file.m_Header;

    writeln!(out, "serialized file")?;
    writeln!(
        out,
        "  unity version: {}",
        file.m_UnityVersion
            .as_ref()
            .map_or_else(|| "<unknown>".to_owned(), |v| v.to_string())
    )?;
    writeln!(out, "  format version: {}", header.m_Version)?;
    writeln!(out, "  endianness: {:?}", header.m_Endianess)?;
    writeln!(out, "  file size: {}", header.m_FileSize)?;
    writeln!(out, "  type tree: {}", file.m_EnableTypeTree)?;
    writeln!(out, "  types: {}", file.m_Types.len())?;
    writeln!(out, "  objects: {}", file.objects().len())?;
    writeln!(out, "  externals: {}", file.m_Externals.len())?;
    Ok(())
}

/// List the files contained in an asset bundle.
fn bundle_info(ctx: &Ctx, out: &mut impl Write) -> Result<()> {
    let bundle = ctx.open_bundle()?;

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

/// Summary of a unity game directory.
fn game_info(ctx: &Ctx, out: &mut impl Write) -> Result<()> {
    let env = ctx.env();

    let unity_version = env
        .unity_version()
        .map_or_else(|e| format!("<unknown: {e}>"), |v| v.to_string());
    let serialized = env.game_files.serialized_files()?.len();
    let (addressables, bundles) = match env.addressables() {
        Ok(Some(_)) => ("yes", env.addressables_bundles().map(|b| b.len()).ok()),
        Ok(None) => ("no", None),
        Err(_) => ("error", None),
    };

    writeln!(out, "game directory")?;
    writeln!(out, "  unity version: {unity_version}")?;
    writeln!(out, "  serialized files: {serialized}")?;
    writeln!(out, "  addressables: {addressables}")?;
    if let Some(bundles) = bundles {
        writeln!(out, "  addressables bundles: {bundles}")?;
    }
    Ok(())
}
