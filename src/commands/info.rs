use anyhow::Result;

use crate::ctx::Ctx;
use crate::target::Target;

pub fn run(ctx: &Ctx) -> Result<()> {
    match &ctx.target {
        Target::SerializedFile(path) => {
            // TODO: parse file, print unity version, object count, class histogram.
            println!("serialized file: {}", path.display());
        }
        Target::Bundle(path) => {
            // TODO: open bundle, print contained serialized files / block info.
            println!("bundle: {}", path.display());
        }
        Target::GameDir(path) => {
            // TODO: app name, unity version, file counts via ctx.env().
            println!("game dir: {}", path.display());
        }
    }

    Ok(())
}
