# Tree milestone 1: the source tree, made approachable

Status: **planned 2026-11-21** (Dave: "do the space preview fix, then the
tree work"), **answered the same night**: width 120, the quotes move to the
docs, CLAUDE.md split, one STATUS.md (all four as Claude leaned). Starts
after preview milestone 1. This is part 2 of
docs/release-pathway.md, done before the first package so the source a package
points at is the source people will read.

## Why

Claude's review, 2026-11-21, graded the tree B for clarity and C+ for
approachability. The code reads well, but getting started depends on knowing
the lab: the host names, the dated conversations in comments, and status
spread over the whitepaper's §9, the README, CLAUDE.md and 55 docs. Someone
who clones the repository should be able to find the parts, build, run a
nested session and run the tests in an afternoon, without asking us.

## Where it stands (measured 2026-11-21)

- 38 crates and 139,000 lines of Rust. 7,700 lines run over 100 characters,
  4,100 over 120, 1,700 over 150 and 400 over 200. Most of these are struct
  literals and builder chains on one line.
- There is no rustfmt.toml, and the tree has never been formatted as a whole.
- Big files: Settings' main.rs (3,900 lines), FileView's main.rs (3,600), the
  terminal's main.rs (2,800), widgets' ui.rs (2,600) and textarea.rs (2,400),
  vergence-ui lib.rs (2,200), vdm.rs (2,100), and Reader, the shelf and Edit
  at about 1,900 each.

## Steps

Mechanical steps come first, so the hand work after them is done on the
final form, and each step lands as its own commits.

1. **One format.** rustfmt.toml with one width, the tree run through `cargo
   fmt` once, in a commit that does nothing else (listed in
   `.git-blame-ignore-revs`, so blame skips it). `cargo fmt --check` joins
   the checks on Hubble.
2. **Big files split by part**, with no behavior change:
   - Settings' main.rs by page, as wifi.rs and places.rs already are.
   - FileView's main.rs: the tab, the sidebar's actions, the preview, the
     drives and mounting, the watch.
   - The terminal, Reader and Edit where a part stands alone.
   - The tests and the display suite must pass unchanged after each split.
3. **Comments keep the why, not the dialogue.** A comment states the rule and
   its reason, then points to the doc ("see docs/drives-milestone-1.md").
   Dated quotes ("Dave, 2026-11-21: ...") move to the milestone docs, which
   already hold most of them, or to a decisions log. Done crate by crate.
4. **Lab tools set apart.** sync-hubble.sh, fetch-gpu-reports.sh,
   test-guard.sh, the display-check host defaults and the lab tiles move to
   tools/lab/. The host names leave anything a contributor runs.
5. **The documents a newcomer reads**:
   - **README.md**, for people: what Vergence is, screenshots, status, and
     how to try it.
   - **ARCHITECTURE.md**: one diagram (the server, the bus, the toolkit, the
     apps, vdm), then a tour: how an app gets a window, how a frame reaches
     the screen, where input goes, and what each crate is for.
   - **CONTRIBUTING.md**: build and run on one FreeBSD or Linux box (`cargo
     build`, a nested session with tools/vergence-session), the tests, the
     display suite, and the rules (security, borrowing, license, American
     spelling, the bus version lockstep).
   - **STATUS.md**, the one status page: what is built, what is open, and
     what is next. The whitepaper's §9, the README and CLAUDE.md point to it
     instead of each keeping their own copy.
   - **A crate doc in each crate** (the `//!` at the top of lib.rs or
     main.rs): what it is for, what it uses, and where to start reading.
6. **Checks:** the workspace tests and the display suite on Hubble and
   techbench, clippy clean, `cargo fmt --check` clean, and `cargo doc`
   without warnings. Then a cold read: a fresh agent, given only the
   repository, builds it, runs a nested session and finds three named
   things in the code. What it trips over is fixed.

## Not in it

Packaging (the FreeBSD port, the xbps template), going public, and history
rewriting all come after this, in release-pathway's order. Renaming crates
and changing behavior are out too.

## Questions for Dave (answered 2026-11-21: all four as leaned)

1. **Line width.** Rust's default is 100. At 100, rustfmt breaks about 7,700
   lines and the code gets taller. At 120, about 4,100 lines are touched and
   it reads closer to the code today. Claude's lean: **120**.
2. **The dated quotes in comments.** Move them to the milestone docs and keep
   a one-line rule plus a pointer in the code (Claude's lean)? Or keep the
   short ones in place, since they say why in your words?
3. **CLAUDE.md.** It holds both the project's rules and the lab's details
   (host names, who has root where, test machines). Split it in two:
   CONTRIBUTING.md takes the rules anyone needs, and CLAUDE.md keeps the lab
   and points to CONTRIBUTING.md (Claude's lean)? Or leave it whole for now
   and decide when going public?
4. **STATUS.md replacing the whitepaper's §9 status:** §9 keeps the order
   of work and the reasons, and STATUS.md holds the state (Claude's lean)?
