use anyhow::Result;

use crate::cli::LsArgs;
use crate::ctx::Ctx;

pub fn run(ctx: &Ctx, args: LsArgs) -> Result<()> {
    let file = ctx.load()?;

    for obj in file.objects::<()>() {
        let class_id = obj.class_id();
        if let Some(filter) = &args.r#type
            && format!("{class_id:?}") != *filter
        {
            continue;
        }
        println!("{:>12}  {:?}", obj.path_id(), class_id);
    }

    Ok(())
}
