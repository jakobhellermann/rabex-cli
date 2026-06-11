//! Verbs that operate on a single serialized file (shared by the `file`,
//! `scene` and `bundle <path> file` commands).

use std::io::Write;

use anyhow::Result;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;
use rabex_env::unity::types::Transform;

use crate::cli::{FileVerb, Format, ObjArgs};

pub fn run<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    verb: FileVerb,
) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match verb {
        FileVerb::Info => info(file, &mut out),
        FileVerb::Ls(args) => list(file, args.r#type.as_deref(), &mut out),
        FileVerb::Obj(args) => dump(file, args, &mut out),
        FileVerb::Tree => tree(file, &mut out),
    }
}

/// Header information + type count for a serialized file.
pub fn info<R: EnvResolver, P: TypeTreeProvider>(
    handle: &SerializedFileHandle<'_, R, P>,
    out: &mut impl Write,
) -> Result<()> {
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

/// Write `path_id  ClassId` for each object, optionally filtered to a single
/// class name. Generic over the resolver so tests can drive it with an
/// in-memory file.
pub fn list<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    type_filter: Option<&str>,
    out: &mut impl Write,
) -> Result<()> {
    for obj in file.objects::<()>() {
        let class_id = obj.class_id();
        if let Some(filter) = type_filter
            && format!("{class_id:?}") != *filter
        {
            continue;
        }
        writeln!(out, "{:>12}  {:?}", obj.path_id(), class_id)?;
    }
    Ok(())
}

/// Read object `path_id` via its typetree and write it in the requested format.
pub fn dump<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    args: ObjArgs,
    out: &mut impl Write,
) -> Result<()> {
    dump_path_id(file, args.path_id, args.format, out)
}

/// As [`dump`], by raw path id — convenient for tests.
pub fn dump_path_id<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    path_id: PathId,
    format: Format,
    out: &mut impl Write,
) -> Result<()> {
    let object = file.object_at::<serde_json::Value>(path_id)?;
    let value = object.read()?;

    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut *out, &value)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

/// Print the GameObject hierarchy: each root transform (no parent) and its
/// children recursively, indented by depth. Names come from each transform's
/// GameObject; the GameObject path id is shown for `obj` follow-up.
pub fn tree<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    out: &mut impl Write,
) -> Result<()> {
    for transform in file.transforms() {
        let transform = transform.read()?;
        if transform.m_Father.optional().is_some() {
            continue;
        }
        print_node(file, &transform, 0, out)?;
    }
    Ok(())
}

fn print_node<R: EnvResolver, P: TypeTreeProvider>(
    file: &SerializedFileHandle<'_, R, P>,
    transform: &Transform,
    depth: usize,
    out: &mut impl Write,
) -> Result<()> {
    let go = transform.m_GameObject;
    let name = file.deref_read(go)?.m_Name;
    writeln!(out, "{}{}  #{}", "  ".repeat(depth), name, go.m_PathID)?;

    for child in &transform.m_Children {
        let child = file.deref_read(*child)?;
        print_node(file, &child, depth + 1, out)?;
    }
    Ok(())
}
