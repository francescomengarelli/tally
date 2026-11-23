use crate::cli::Shell;

pub fn run(shell: Shell) {
    match shell {
        Shell::Fish => print!("{FISH}"),
    }
}

/// `claude` goes through tally, keeping claude's completions; account names
/// are read when completing, so a new account needs no new shell.
const FISH: &str = "\
function claude --wraps claude --description 'claude on the account tally picks'
    tally run -- $argv
end
complete -c tally -n '__fish_seen_subcommand_from run login remove' -f -a '(tally accounts)'
";
