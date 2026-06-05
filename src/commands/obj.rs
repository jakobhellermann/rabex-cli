use std::io::Write;

use anyhow::Result;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::objects::pptr::PathId;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;

use crate::cli::{Format, ObjArgs};
use crate::ctx::Ctx;

pub fn run(ctx: &Ctx, args: ObjArgs) -> Result<()> {
    let file = ctx.load()?;
    let stdout = std::io::stdout();
    dump(&file, args.path_id, args.format, &mut stdout.lock())
}

/// Read object `path_id` via its typetree and write it in `format`. Generic
/// over the resolver so tests can drive it with an in-memory file.
pub fn dump<R: EnvResolver, P: TypeTreeProvider>(
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
