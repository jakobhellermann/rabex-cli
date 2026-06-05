use std::io::Write as _;

use anyhow::Result;

use crate::cli::{Format, ObjArgs};
use crate::ctx::Ctx;

pub fn run(ctx: &Ctx, args: ObjArgs) -> Result<()> {
    let file = ctx.load()?;
    let object = file.object_at::<serde_json::Value>(args.path_id)?;
    let value = object.read()?;

    match args.format {
        Format::Json => {
            let mut out = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut out, &value)?;
            writeln!(out)?;
        }
    }

    Ok(())
}
