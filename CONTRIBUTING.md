# Contributing

Bug reports and pull requests are welcome. infiniterm is source-available, not open source, so two things work differently from an MIT project:

- A fork on GitHub is fine for proposing a change back. Publishing builds from it, or using it as a separate app, is not; see `LICENSE.txt`, sections 2 and 3.
- By opening a pull request you grant the licence in section 6 of `LICENSE.txt`: your change can ship in infiniterm, including paid licences, and you keep the copyright to it.

Before a pull request:

1. `make check` passes (fmt, clippy with warnings as errors, every test).
2. New behaviour comes with a test. Logic lives in a pure function with its tests beside it; the gpui side is wiring.
3. The commit message says why the change is needed, not what the diff already shows.
4. Anything a user can see gets a line in `CHANGELOG.md` under `## Unreleased`, in today's section.

`CLAUDE.md` is the long version: where things are, the rules the code follows and why, and the traps already hit. Building needs Rust, the CEF binary distribution and a checkout of cef-rs; the README's "Building from source" has the steps.

Security issues: report them privately from the repo's Security tab ("Report a vulnerability") rather than as an issue.
