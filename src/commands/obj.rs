use anyhow::Result;

use crate::cli::{Format, ObjArgs};
use crate::target::Target;

pub fn run(args: ObjArgs) -> Result<()> {
    let target = Target::detect(&args.path)?;

    match target {
        Target::SerializedFile(path) | Target::Bundle(path) => {
            // TODO: locate object `args.path_id`, read it via its typetree, then emit.
            match args.format {
                Format::Json => {
                    // TODO: serde_json over the typetree value, pretty-printed.
                    println!("{{ /* obj {} from {} */ }}", args.path_id, path.display());
                }
            }
        }
        Target::GameDir(_) => {
            anyhow::bail!("`obj` expects a file or bundle, not a game directory");
        }
    }

    Ok(())
}
