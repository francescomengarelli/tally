# tally

Rust CLI that runs Claude Code on several subscription accounts sharing one
setup, and sums their usage left. Design: `docs/design.md`.

## Before committing

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

All three must be clean.

## Rules

- Tests never hit the network and never touch the real `~/.claude`: use a
  temp home and fake `Net` implementations.
- Error voice: lowercase, first person ("i couldn't ...").
- Commit messages: `type(scope): subject` (feat, fix, chore, docs, test),
  then a one-paragraph description.
- Releases: `just bump patch|minor|major "message"` bumps the version, commits
  and tags; pushing the tag builds the release assets that `install.sh` and
  `tally self-upgrade` download (`tally-<tag>-<target>`).
