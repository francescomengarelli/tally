# tally — design

Several Claude subscriptions on one machine, one shared Claude Code setup,
and one number for how much is left across all of them.

## Goals

- Run Claude Code as any of several accounts, each with its own login.
- Share the Claude Code setup (settings, skills, plugins, memory, sessions)
  between accounts.
- Show how much usage is left, summed over all accounts, in one unit the
  user already understands.
- Pick the best account automatically when none is named.

## Non-goals

- **Not a dotfiles manager.** tally never touches the dotfiles repo and never
  links anything outside its own account folders. Dotfiles stay perch's job.
- No secrets in config. Logins live only in account folders.
- No guessing token counts or dollar costs. Anthropic publishes plan
  multiples, not absolute amounts.

## Accounts

An account is a Claude Code config folder, the one `CLAUDE_CONFIG_DIR`
points to. Accounts are declared in the config, which lives in the dotfiles:

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

The main account (the one whose `dir` is `~/.claude`) runs without
`CLAUDE_CONFIG_DIR`, so Claude keeps using `~/.claude.json` for it.

With no config file, tally treats `~/.claude` as a single account named
`main`.

### Shared entries are derived, not managed

For every account other than the main one, each entry in `shared` that
exists in `~/.claude` becomes a symlink `<account dir>/<entry>` →
`~/.claude/<entry>`. The links are a consequence of the config, not
something the user maintains:

- tally reconciles them on every `run`, `login` and `add`: it creates
  missing links, fixes wrong ones, and removes its own links for entries no
  longer listed. This is cheap (a few `readlink` calls).
- A real file or folder where a link should be is never overwritten. tally
  warns and leaves it.
- Links target `~/.claude/<entry>`, never the dotfiles repo, and live only
  under account folders. perch only touches links in its own manifest, so the
  two never overlap.
- Deleting an account folder loses only its login. Everything else comes
  back on the next run.

`.claude.json` is per account. `tally login` seeds a new one with the main
account's `mcpServers` and `theme`, so user MCP servers carry over. MCP
logins (in `.credentials.json`) are per account and must be done once each.

## Commands

```
tally                         usage per account, then totals
tally run [ACCOUNT] [-- ARGS] run claude as ACCOUNT, or the picked one
tally pick [--why]            print the picked account (and the reasoning)
tally add NAME [--plan X]     add the account to the config, then log in
tally remove NAME             remove it from the config; ask before deleting its folder
tally login ACCOUNT           log the account in (creates its folder if needed)
tally notice                  one line for shell startup
tally statusline              one line for Claude Code's status line
tally init fish               claude wrapper and completions, for `| source`
```

- `run`: after `run`, the only positional is an account name. Everything
  after `--` goes to claude untouched. tally `exec`s claude with
  `CLAUDE_CONFIG_DIR` (unless main) and `TALLY_ACCOUNT=<name>`.
- `add` edits the config in place with `toml_edit`, keeping comments and
  layout, writing through the symlink perch made.
- `login` runs `claude auth login` with the account's `CLAUDE_CONFIG_DIR`.
- `init fish` prints a `claude` function (`tally run`) that wraps claude's
  completions, and completes account names for `run`, `login` and `remove`
  through the hidden `tally accounts`, read at completion time. It reads no
  config.

### New machine

1. `perch sync` restores the tally config, the startup line and the
   status line setting.
2. `tally login <account>` once per account (the main one is already logged
   in if Claude was).
3. The shared links appear on first run.

An account without a login shows as `not logged in → tally login <name>` and
is never picked.

## Usage data

For each account, in parallel (one thread each, scoped):

1. Read `<dir>/.credentials.json` → `claudeAiOauth` (`accessToken`,
   `refreshToken`, `expiresAt`, `rateLimitTier`).
2. If the access token has **already** expired, refresh it (below).
3. `GET https://api.anthropic.com/api/oauth/usage` with
   `Authorization: Bearer <token>` and `anthropic-beta: oauth-2025-04-20`.
   The fields used are `five_hour.utilization`, `five_hour.resets_at`,
   `seven_day.utilization` and `seven_day.resets_at`. A missing `five_hour`
   means the plan has no 5-hour limit (Ultra, verified).
4. Store the result in `~/.cache/tally/<account>.json` with a timestamp.

Each command passes a maximum cache age: `notice` 5 min, `pick` 1 min,
`statusline` reads the cache only and refreshes in the background when it is
older than 1 min. Network calls time out after 3 s; an account that fails
keeps its last cached value, marked stale.

If a `resets_at` is in the past, its utilization counts as 0.

This endpoint is the one Claude Code's `/usage` uses. It is not publicly
documented and may change. If it fails, tally shows the account as
unavailable and `run` still works, picking without data.

### Token refresh

Access tokens live 8 hours. Refresh tokens rotate on use: the old one stops
working.

- Refresh only when the access token has already expired. A running Claude
  session refreshes before expiry, so tally never competes with an active
  session.
- `POST https://platform.claude.com/v1/oauth/token` with
  `{grant_type: "refresh_token", refresh_token, client_id:
  "9d1c250a-e61b-44d9-88ed-5944d1962f5e"}`.
- Update `accessToken`, `refreshToken` (if returned) and `expiresAt`, keep
  every other field, and write atomically (temp file in the same folder,
  mode 0600, rename).
- If the refresh is rejected, re-read the file once (Claude may have just
  refreshed). If it is still rejected, show `login expired → tally login
  <name>`.

## Units

Everything is shown in **% of the unit account's week**, the number the user
already reads in `/usage`.

Plan multiples of Pro, from Anthropic's published figures:

| Plan | Week | 5 hours |
| --- | --- | --- |
| Pro | 1 | 1 |
| Max | 5 | 5 |
| Team Premium | 6.25 | 6.25 |
| Ultra | 10 | none (verified accounts) |

`plan` in the config wins. Without it, the multiple comes from
`rateLimitTier` (`…max_5x` → 5, `…max_20x` → 10). Anything unknown counts as
1 with a warning. Team Premium reports `max_5x`, so it needs `plan = 6.25`.

For an account `a` with weekly multiple `P_a`, and the unit account's `P_u`:

- `weight_a = P_a / P_u`
- `week_a = (100 − seven_day.utilization) × weight_a`
- `five_a = (100 − five_hour.utilization) × five_hour × weight_a`, or
  unlimited when there is no 5-hour limit
- `now_a = min(five_a, week_a)`: what can be spent before hitting a limit
- `days_a = max(days until seven_day.resets_at, 1)`
- `day_a = week_a / days_a`: the daily rate that uses the week up before it
  resets. With less than a day left, all of it counts as today.

Totals are plain sums. `week` also shows the total capacity
(`100 × Σ weight`), e.g. `week 206% left (of 260%)`.

## Picking an account

Unused weekly usage is lost at reset, so spend first what expires first, but
only where a session can actually run.

1. **Eligible:** logged in, data available, and `now_a ≥ min_session`.
2. **Among eligible, the most urgent:** highest `day_a`. Ties go to the
   larger `now_a`.
3. **None eligible:** the account with the largest `now_a`, with a warning
   saying when the first limit frees up.
4. **No data at all** (offline): the unit account.

`tally pick --why` prints each account's `now`, `week`, days to reset and
`day`, and which rule decided.

## Output

`notice` (once per kitty OS window, from a fish `conf.d` file in the
dotfiles):

```
   claude: now 72% · week 206% left (of 260%) · 31%/day (37% steady)  →  claude (personal)
```

`statusline`, from `TALLY_ACCOUNT`, or from `CLAUDE_CONFIG_DIR` when Claude
was started without tally:

```
personal · now 72% · week 206% · 31%/day (37%)
```

When the picked account is not the one running, and switching is worth it,
the line ends with `  →  tally run <picked> -- -r <session id>`, the session
id read from the JSON Claude Code pipes in (letters, digits and dashes only;
without one, just `tally run <picked>`). Worth it means:

- **red:** the running account has `now < min_session` and the picked one
  has `now ≥ min_session`;
- **yellow:** both have room and the picked one's `day` is at least 1.5 times
  the running one's. Below that, re-reading the session on the new account
  costs more than it gains.

`tally` with no command prints one row per account (plan, raw 5h and week %,
resets, `now`, `week`, `/day`, `steady`) and a totals row.

`steady` is `capacity / 7`. Colours: `/day` green at ≥ 1.2 × steady, red at
≤ 0.8 × steady; `now` red below `min_session`, yellow below twice that;
`week` red below 15% of capacity, yellow below 35%. `NO_COLOR` turns them off.

Colours only when stdout is a terminal.

## Implementation

Rust, edition 2024, laid out like perch: `cli.rs`, `config.rs`,
`commands/`, plus `usage.rs` (fetch, refresh, cache) and `pick.rs` (pure
maths, unit-tested).

Dependencies: `clap` (derive), `serde`, `toml`, `toml_edit`, `reqwest`
(blocking, json, rustls), `anyhow`, `dirs`, `jiff`. Same size-optimised
release profile, `install.sh` and `self-upgrade` as perch.

Tests: the maths and the pick rules with fixed inputs; link reconciliation in
a temp home; config editing preserves comments; usage parsing from recorded
responses. No network in tests.

## Dotfiles side (done after tally exists)

- `~/.config/tally/config.toml`, managed by perch.
- `common/.config/fish/conf.d/tally.fish`: `tally init fish | source`, and
  `tally notice` once per kitty OS window. The "first shell of this window"
  check is shared with the dotfiles notice.
- `"statusLine": {"type": "command", "command": "tally statusline"}` in the
  shared Claude `settings.json`.
