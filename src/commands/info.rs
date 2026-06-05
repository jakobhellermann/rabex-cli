use anyhow::Result;

use crate::cli::InfoArgs;
use crate::target::Target;

pub fn run(args: InfoArgs) -> Result<()> {
    let target = Target::detect(&args.path)?;

    match target {
        Target::SerializedFile(path) => {
            // TODO: parse file, print unity version, object count, class histogram.
            println!("serialized file: {}", path.display());
        }
        Target::Bundle(path) => {
            // TODO: open bundle, print contained serialized files / block info.
            println!("bundle: {}", path.display());
        }
        Target::GameDir(path) => {
            // TODO: GameFiles::probe + Environment, print app name, unity version, file counts.
            println!("game dir: {}", path.display());
        }
    }

    Ok(())
}
