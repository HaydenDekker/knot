# Phase 5: Migrate this repo's rig, bump, verify

**Plan:** [Rig Repository Separation](rig-repo-separation-plan.md)

## Checklist
- [ ] Bump `Cargo.toml` version to `0.31.0` (breaking layout change)
- [ ] `cargo build` (release)
- [ ] Run the new binary against this repo's own rig — verify auto-migration: `rig/tie-offs/` + `rig/state.json` + `rig/.rig-log` + `rig/events/` → `tie-offs/rig/`
- [ ] Rig git initialised (`rig/.git` exists) and parent `.gitignore` has the marked `rig/` entry
- [ ] Remove legacy empty `rig/output/` dirs
- [ ] Manual `git rm -r --cached rig/` and commit the project side
- [ ] `cargo install --path .`
- [ ] Smoke pass: fresh temp project → run knot → rig git exists, parent `.gitignore` updated, rig dir source-only
- [ ] Smoke pass: trigger a strand → tie-off/loom-log/state land under `tie-offs/<rig>/` and the Knot commit touches them while `rig/` stays out of the commit
- [ ] Commit phase 5 record

## Deviations

## Discoveries

## Notes
