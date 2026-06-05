use std::io::Write;

use anyhow::Result;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;

use crate::cli::LsArgs;
use crate::ctx::Ctx;

pub fn run(ctx: &Ctx, args: LsArgs) -> Result<()> {
    let file = ctx.load()?;
    let stdout = std::io::stdout();
    list(&file, args.r#type.as_deref(), &mut stdout.lock())
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
