use anyhow::Result;

use crate::cli::Shell;
use crate::context::Context;

pub fn run(ctx: &Context, shell: Shell) -> Result<()> {
    match shell {
        Shell::Fish => print!("{}", fish(ctx)),
    }
    Ok(())
}

fn fish(ctx: &Context) -> String {
    let mut out = String::from(
        "function cl --wraps claude --description 'claude on the account tally picks'\n    tally run -- $argv\nend\n",
    );
    for a in &ctx.accounts {
        let name = &a.name;
        out.push_str(&format!(
            "function cl-{name} --wraps claude --description 'claude as {name}'\n    tally run {name} -- $argv\nend\n"
        ));
    }
    out
}
