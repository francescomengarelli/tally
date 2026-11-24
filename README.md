![GitHub Release](https://img.shields.io/github/v/release/francescomengarelli/tally?display_name=release&style=flat-square&color=blue)
![CI](https://img.shields.io/github/actions/workflow/status/francescomengarelli/tally/ci.yml?style=flat-square)

# tally

one claude code setup, several subscriptions, one number for what's left.

## Installation

```sh
curl -fsSL https://get.mengarelli.eu/tally | sh
```

Or build from source:

```sh
git clone git@github.com:francescomengarelli/tally.git
cd tally
cargo install --root ~/.local --path .
```

Update later with `tally self-upgrade`.

## How it works

An account is a Claude Code config folder, the one `CLAUDE_CONFIG_DIR` points
to. Declare them in `~/.config/tally/config.toml`; without a config, tally
treats `~/.claude` as a single account named `main`.

### Shared setup

For every account other than the main one, each entry in `shared` that exists
in `~/.claude` (settings, skills, plugins, memory, sessions...) becomes a
symlink into `~/.claude`. The links are derived from the config, not managed
by hand: tally creates, fixes and removes them on every `run`, `login` and
`add`, and never overwrites a real file or folder. Deleting an account folder
loses only its login.

### Units

Everything is shown in **% of the unit account's week**, the number you
already read in `/usage`. Other plans are weighted by their multiple of Pro
(Max 5x counts 5, Team Premium 6.25, Ultra 10), taken from the login or from
`plan` in the config.

- `now`: what you can spend before hitting a limit (the smaller of the 5-hour
  window and the week)
- `week`: what is left of the week, summed over accounts
- `/day`: the daily rate that uses the week up before it resets
- `steady`: the week spread evenly (capacity / 7), what a fresh week allows per day.
  The lines show it after `/day`: `31/37%/day` is 31% against a steady 37%

Numbers are coloured: `/day` is green above steady (usage that resets soon,
spend it) and red below it (the week went early, go easy); `now` is red when
there isn't room for a session and yellow when it's tight; `week left` goes
yellow and red as the week runs out. Set `NO_COLOR` to turn colours off.

A multiple shown as `1x?` means the plan was guessed because the login's tier
is unknown: set `plan` for that account. Team Premium seats report no clear
tier, so give them `plan = 6.25`.

### Picking

Unused weekly usage is lost at reset, so tally spends first what expires
first, but only where a session can run: among accounts that are logged in
and have at least `min_session` available now, it picks the one with the
highest `/day`. If none qualifies it picks the one with the most `now`; with
no data at all it picks the unit account. `tally pick --why` shows the
reasoning.

## Configuration

```toml
# ~/.config/tally/config.toml
unit = "acme"          # the account whose plan reads as 100%
five_hour = 0.18         # one full 5-hour window = 18% of that plan's week
min_session = 3          # % (in unit) an account needs to be picked
shared = ["CLAUDE.md", "settings.json", "keybindings.json", "skills", "agents",
          "commands", "plugins", "projects", "plans", "history.jsonl",
          "file-history"]

[account.acme]
dir = "~/.claude"        # the main folder, used as is
plan = 6.25              # multiple of Pro; overrides what the login says

[account.personal]       # dir defaults to ~/.local/share/tally/personal
```

A `dir` may start with `~` or `$HOME`. It must be a folder of its own: not
home, not a parent of `~/.claude`, and not shared with another account.

## Commands

| Command                        | What it does                                        |
| ------------------------------ | --------------------------------------------------- |
| `tally`                        | Usage per account, then totals                      |
| `tally run [ACCOUNT] [-- ARGS]`| Run claude as ACCOUNT, or the picked one            |
| `tally pick [--why]`           | Print the picked account (and the reasoning)        |
| `tally add NAME [--plan X]`    | Add the account to the config, then log it in       |
| `tally remove NAME`            | Remove it from the config; ask before deleting its folder |
| `tally login ACCOUNT`          | Log the account in (creates its folder if needed)   |
| `tally notice`                 | One line for shell startup                          |
| `tally statusline`             | One line for Claude Code's status line              |
| `tally init fish`              | `claude` wrapper and completions, for `\| source`   |
| `tally self-upgrade`           | Upgrade to the latest release                       |

## Integrations

### Shell and kitty

In `~/.config/fish/conf.d/tally.fish`:

```fish
tally init fish | source
```

This makes `claude` run `tally run`, keeping claude's completions, and
completes account names after `tally run`, `login` and `remove`. `command
claude` still runs the binary as-is. Call `tally notice` to print the startup line. The
once-per-kitty-window check lives in your shell config (your dotfiles), not in
tally:

```
 󰚩  now 72% · week 206/260% · 31/37%/day → personal
```

### Status line

In Claude Code's `settings.json`:

```json
"statusLine": { "type": "command", "command": "tally statusline" }
```

It prints the totals over all accounts, labelled with the account that is
running (`TALLY_ACCOUNT`, else the account whose folder is `CLAUDE_CONFIG_DIR`,
else the main one):

```
personal · now 72% · week 206% · 31/37%/day
```

When another account is worth switching to, it ends with the command that
resumes this session there:

```
acme · now 2% · week 40% · 13/37%/day → tally run personal -- -r 3f2a91c0-…
```

Red means this account has no room left for a session, yellow that the other
one has at least 1.5 times as much to spend per day. Switching re-reads the
whole session on the new account, so it is best done between tasks.

It prints nothing until there is cached data, and reads only the cache: a
background refresh keeps it fresh.

## Troubleshooting

- Background refreshes write their errors to `~/.cache/tally/refresh.log`.
- A login the server has refused shows as "to log in" until you run
  `tally login <account>`.
- Every command takes a global `--config <file>` to use another config file.

## New machine

1. Restore the config (for example with perch).
2. `tally login <account>` once per account.
3. The shared links appear on first run.
