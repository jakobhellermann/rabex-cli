//! The `bundle` command: inspect one bundle's contents and drill into a
//! contained serialized file. (`bundles` lists all bundles.)

use std::io::{Cursor, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use rabex_env::Environment;
use rabex_env::env::Data;
use rabex_env::rabex::files::bundlefile::{BundleFileReader, CompressionType};
use rabex_env::rabex::files::unityfile::FileEntry;
use serde::Serialize;

use crate::cli::{BundleArgs, BundleVerb, Context, Format};
use crate::commands::file::{self, FileLocation};
use crate::ctx;
use crate::output::{Render, emit, style};

pub fn run(game: &Context, args: BundleArgs, format: Format) -> Result<()> {
    let (env, bundle) = ctx::open_bundle(game, &args.path)?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match args.verb.unwrap_or(BundleVerb::Info) {
        BundleVerb::Info => emit(&info(&bundle)?, format, &mut out),
        BundleVerb::Files => emit(&list_files(&bundle), format, &mut out),
        BundleVerb::File(file_args) => {
            // Default to the bundle's main serialized file when no CAB is given.
            let cab = match file_args.cab {
                Some(cab) => cab,
                None => bundle
                    .main_serializedfile()
                    .context("bundle contains no serialized file")?
                    .path
                    .clone(),
            };
            let handle = ctx::bundle_serialized(&env, &bundle, Some(&cab))?;
            let source = FileLocation::Bundle { cab };
            file::run_verb(source, &handle, file_args.verb, format)
        }
    }
}

/// Every addressables bundle in the game (`bundles`).
#[derive(Serialize)]
#[serde(transparent)]
pub struct Bundles(pub Vec<PathBuf>);

impl Render for Bundles {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for bundle in &self.0 {
            writeln!(out, "{}", bundle.display())?;
        }
        Ok(())
    }
}

/// List every addressables bundle in the game (`bundles`).
pub fn list_all(env: &Environment, format: Format) -> Result<()> {
    let bundles = Bundles(env.addressables_bundles()?);
    let stdout = std::io::stdout();
    emit(&bundles, format, &mut stdout.lock())
}

/// The files (CABs) contained in a bundle.
#[derive(Serialize)]
#[serde(transparent)]
pub struct BundleFiles(pub Vec<String>);

impl Render for BundleFiles {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        for path in &self.0 {
            writeln!(out, "{path}")?;
        }
        Ok(())
    }
}

/// List the files (CABs) contained in a bundle.
fn list_files(bundle: &BundleFileReader<Cursor<Data>>) -> BundleFiles {
    BundleFiles(bundle.files().iter().map(|e| e.path.clone()).collect())
}

/// One contained file in a [`BundleInfo`].
#[derive(Serialize)]
pub struct BundleEntry {
    size: u64,
    kind: &'static str,
    path: String,
}

/// The bundle container header.
#[derive(Serialize)]
pub struct BundleHeader {
    signature: String,
    version: u32,
    unity_version: String,
    /// `None` when the bundle was built with a stripped version (`0.0.0`).
    unity_revision: Option<String>,
    size: u32,
}

/// One storage block of the bundle's data stream.
#[derive(Serialize)]
pub struct BundleBlock {
    compressed_size: u32,
    uncompressed_size: u32,
    compression: &'static str,
    flags: u32,
}

/// The bundle's header, storage blocks and contained files with sizes and kinds.
#[derive(Serialize)]
pub struct BundleInfo {
    header: BundleHeader,
    blocks: Vec<BundleBlock>,
    files: Vec<BundleEntry>,
}

impl Render for BundleInfo {
    fn render(&self, out: &mut dyn Write) -> Result<()> {
        let header = &self.header;
        writeln!(
            out,
            "{} v{}  unity {}  revision {}  {} bytes",
            style::header(&header.signature),
            header.version,
            header.unity_version,
            header
                .unity_revision
                .as_deref()
                .unwrap_or("0.0.0 (stripped)"),
            header.size,
        )?;

        let compressed: u64 = self.blocks.iter().map(|b| b.compressed_size as u64).sum();
        let uncompressed: u64 = self.blocks.iter().map(|b| b.uncompressed_size as u64).sum();

        let mut by_compression: Vec<(&str, usize)> = Vec::new();
        for block in &self.blocks {
            match by_compression
                .iter_mut()
                .find(|(c, _)| *c == block.compression)
            {
                Some((_, n)) => *n += 1,
                None => by_compression.push((block.compression, 1)),
            }
        }

        let by_compression = by_compression
            .iter()
            .map(|(c, n)| format!("{n} {c}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "{} {}  {compressed} -> {uncompressed} bytes",
            style::header(&format!("blocks ({})", self.blocks.len())),
            by_compression,
        )?;

        writeln!(
            out,
            "{}",
            style::header(&format!("bundle ({} files)", self.files.len()))
        )?;
        for entry in &self.files {
            writeln!(
                out,
                "  {}  {}  {}",
                style::dim(&format!("{:>10}", entry.size)),
                style::class(&format!("{:<11}", entry.kind)),
                entry.path
            )?;
        }
        Ok(())
    }
}

fn compression_name(flags: u32) -> Result<&'static str> {
    Ok(match CompressionType::try_from(flags & 0x3F) {
        Ok(CompressionType::None) => "none",
        Ok(CompressionType::Lzma) => "lzma",
        Ok(CompressionType::Lz4) => "lz4",
        Ok(CompressionType::Lz4hc) => "lz4hc",
        Ok(CompressionType::Lzham) => "lzham",
        _ => anyhow::bail!("unknown block compression in flags {flags:#x}"),
    })
}

/// Build the bundle's header, blocks and contained files with sizes and kinds.
fn info(bundle: &BundleFileReader<Cursor<Data>>) -> Result<BundleInfo> {
    let header = bundle.header();
    let header = BundleHeader {
        signature: header.signature.to_string(),
        version: header.version,
        unity_version: header.unity_version.clone(),
        unity_revision: header.unity_revision.as_ref().map(ToString::to_string),
        size: header.size,
    };
    let blocks = bundle
        .blocks()
        .iter()
        .map(|block| {
            Ok(BundleBlock {
                compressed_size: block.compressed_size,
                uncompressed_size: block.uncompressed_size,
                compression: compression_name(block.flags)?,
                flags: block.flags,
            })
        })
        .collect::<Result<_>>()?;
    let files = bundle
        .files()
        .iter()
        .map(|entry| BundleEntry {
            size: entry.size as u64,
            kind: if entry.flags & FileEntry::FLAG_SERIALIZEDFILE != 0 {
                "serialized"
            } else {
                "resource"
            },
            path: entry.path.clone(),
        })
        .collect();
    Ok(BundleInfo {
        header,
        blocks,
        files,
    })
}
