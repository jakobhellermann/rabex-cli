use std::io::Write;

use anyhow::Result;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::rabex::typetree::TypeTreeProvider;
use rabex_env::resolver::EnvResolver;

use crate::cli::LsArgs;
use crate::ctx::Ctx;
use crate::target::Target;

pub fn run(ctx: &Ctx, args: LsArgs) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // On a whole game, `ls` lists the game's serialized files and addressables
    // bundles; on a file or bundle it lists the contained objects.
    match &ctx.target {
        Target::GameDir(_) => {
            let env = ctx.env();
            for path in env.game_files.serialized_files()? {
                writeln!(out, "{}", path.display())?;
            }
            // Addressables bundles, if the game has any.
            for bundle in env.addressables_bundles().unwrap_or_default() {
                writeln!(out, "{}", bundle.display())?;
            }
            Ok(())
        }
        _ => {
            let file = ctx.load()?;
            list(&file, args.r#type.as_deref(), &mut out)
        }
    }
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
