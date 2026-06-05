use anyhow::Result;

use crate::cli::LsArgs;
use crate::target::Target;

pub fn run(args: LsArgs) -> Result<()> {
    let target = Target::detect(&args.path)?;

    match target {
        Target::SerializedFile(path) | Target::Bundle(path) => {
            // TODO: iterate objects, print `<path_id>  <ClassId>  <name?>`,
            //       filtered by `args.r#type` if given.
            let _ = &args.r#type;
            println!("ls: {}", path.display());
        }
        Target::GameDir(_) => {
            anyhow::bail!("`ls` expects a file or bundle, not a game directory");
        }
    }

    Ok(())
}
