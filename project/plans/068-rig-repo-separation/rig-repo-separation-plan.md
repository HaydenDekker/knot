# Plan: Rig Repository Separation — Own Git for the Rig, Project-Level Runtime Tree

## Problem

The rig couples two concerns that belong to different ownership domains:

1. **Project-specific runtime data lives inside the rig.** `rig/tie-offs/`
   holds tie-off files (`{loom-id}/tie-off-{knot-name}.md`), dispatched
   event strands (`{loom-id}/{EventId}/event-*.md`), and per-loom activity
   logs (`{loom-id}/.loom-log`); `rig/state.json`, `rig/.rig-log`, and
   `rig/events/` add live state and the disk-backed queue. All of these are
   project data — they reference project strands, project events, and
   project history — but they sit inside the rig, which is the reusable,
   generic orchestration source.

2. **The rig has no identity of its own in version control.**
   `FileSystemGitVersioner` runs `git add -A` + `git commit` at the
   *project root* after every successful knot run. Everything the rig
   contains (definitions, output, logs, state) is swept into the project's
   git history. The rig source cannot be versioned, reviewed, or shared
   independently — it is entangled with the project's history, which
   defeats reusability.

The desired end state (per the project owner):

- The **rig's project-side runtime tree (tie-offs and all runtime
  artifacts) is committed with the project git** — it is project output
  and belongs to the project's audit history. Capturing loom-logs,
  rig-log, the event queue, and state snapshots in the same commits gives
  a single, clear audit trail per knot run.
- The **rig is initialised with its own git repository**, and the user
  commits the rig git manually. The rig git tracks exactly the reusable
  rig source (looms, knots, profiles, config) — no runtime data at all.
- The nested rig repository **must not leak into the parent repo's
  commits**.

### Verified git behaviour (tested 2026-08-17)

Nesting a repo inside a parent repo works, but with two important
caveats that shape the design:

| Scenario | Parent `git add -A` behaviour | Mitigation |
|---|---|---|
| **Fresh** — `rig/` never tracked by parent, `rig/.git` exists | Stages `rig` as a **gitlink** (mode 160000) with an "embedded git repository" warning. Clones of the parent silently lose the rig's contents. | `rig/` entry in the parent's `.gitignore` — `git add -A` then skips it entirely. |
| **Existing** — `rig/` files already tracked by parent (this repo's case) | Parent **keeps tracking** files under `rig/` despite the nested `.git` and despite a `.gitignore` entry. Changes under `rig/` continue to be staged and committed by the parent. | One-time manual `git rm -r --cached rig/` to untrack, *then* the `.gitignore` entry holds. The Knot binary must **not** run `git rm` — untracking rewrites the project's index/history policy. |

Consequences:

- Rig init must both create `rig/.git` **and** exclude `rig/` from the
  parent repo (when the project root is inside a git repo).
- Existing projects need a one-time manual untrack step, documented in the
  `knot-update` changelog and the `knot-init` skill.

## Target

When this plan is complete:

```
<project-root>/
├── rig/                             ← pure source; its own git repo
│   ├── .git/                        ←   (Knot inits, user commits manually)
│   ├── .workspace-agent-config.yaml ← tracked by rig git
│   ├── profiles/                    ← tracked by rig git
│   └── {name}-loom/*.md             ← tracked by rig git
└── tie-offs/                        ← project git: everything the rig does
    └── {rig-basename}/              ←   at runtime, per rig
        ├── state.json               ← live rig state (5s snapshot)
        ├── .rig-log                 ← rig operational log (JSONL)
        ├── events/                  ← disk-backed strand event queue
        │   └── {event-id}.json
        └── {loom-id}/
            ├── .loom-log            ← per-loom activity log (JSONL)
            ├── tie-off-{knot-name}.md
            └── {EventId}/event-{timestamp}.md
```

Behavioural contract:

1. **Runtime root** — one new domain derivation,
   `derive_runtime_root(rig_dir)` =
   `<project-root>/tie-offs/<rig-basename>/` where `<project-root>` =
   parent of the rig directory (existing convention) and
   `<rig-basename>` = rig directory file name (`rig`, `dev-rig`, …).
   Everything below the root keeps the **current relative layout**:
   tie-off files, event dispatch subdirs, and loom-logs sit at
   `{loom-id}/…` exactly as today — only the base moves. Knot files,
   `event:` URIs, and consumer wiring need no frontmatter changes.
2. **All runtime artifacts live at the runtime root** — `state.json`,
   `.rig-log`, `events/`, and the whole `{loom-id}/` tree (tie-offs,
   dispatch dirs, `.loom-log`). The rig directory contains **no runtime
   data at all**, so the rig's git needs no `.gitignore`: it tracks
   exactly the reusable source.
3. **Rig git init** — at startup, if the rig directory lacks a `.git`,
   Knot runs `git init` in it. Knot never commits the rig git — the user
   does, manually. Git-absent environments degrade gracefully (warning,
   rig still runs).
4. **Parent exclusion** — at startup, if the project root is inside a git
   repo, Knot appends a marked `rig/` entry to the parent's `.gitignore`
   (idempotent). If `rig/` is already tracked by the parent, Knot logs a
   warning with the exact `git rm -r --cached rig/` command instead of
   running it.
5. **Git versioning hardening** — the versioner becomes rig-aware: after
   `git add -A` at the project root it runs `git reset -q -- <rig-dir>` so
   an accidentally-staged rig (gitlink or tracked leftovers) can never be
   committed into the project repo. Knot commits now capture, per knot
   run: the agent's work, tie-off append, loom-log append, rig-log,
   queue-file changes, and the state.json snapshot — the unified audit
   trail.
6. **Legacy migration** — at startup, a rig found with the old layout is
   migrated automatically (idempotent, logged): the entire
   `rig/tie-offs/` tree moves to `tie-offs/<rig-basename>/` (subtree
   preserved), and `rig/state.json`, `rig/.rig-log`, `rig/events/` move to
   the runtime root. The rig directory is left with source only.
7. **Skills, glossary, docs, and the `knot-update` changelog** describe
   the new layout, the two-repo git model, and the one-time migration for
   existing projects. Version bumps to 0.31.0.

## Design Options

### Option A — Project-level runtime tree + rig's own git (recommended)

Move the tie-off tree **and all runtime artifacts** (state.json, .rig-log,
events/, loom-logs) to `<project-root>/tie-offs/<rig-basename>/`; the rig
directory becomes pure source with its own git repo (Knot inits, user
commits); `rig/` is excluded from the parent repo.

- ✅ Cleanest ownership split: project data in project git (unified audit
  trail — every Knot commit carries the full before/after picture of a
  knot run), reusable rig source in rig git, runtime state in neither
  repo's *source* history.
- ✅ The rig directory contains zero project data — not even gitignored
  residue. No `.gitignore` needed in the rig git; sharing the rig
  (git clone or `knot share` zip) transfers exactly the reusable source.
- ✅ The `{loom-id}/` subtree shape is unchanged — no knot frontmatter,
  `event:` URI, or consumer wiring changes; only the base path moves.
- ✅ `<rig-basename>` namespacing makes multi-rig projects
  (`knot dev-rig`) collision-free with zero new configuration.
- ✅ The rig directory watcher stops seeing state.json's 5-second rewrite
  churn (less config-pipeline noise).
- ⚠️ While Knot runs, `state.json` rewrites keep the project tree dirty
  (see Notes); the `tie-offs/` name now covers more than tie-off
  documents (glossary wording adjusted, term "tie-off" itself unchanged).
- ⚠️ Largest doc/skill surface of the options (~10 skills, ~10 docs).

### Option B — Keep everything in the rig; rig git tracks output too

No path moves. The rig's git repo tracks looms, profiles, **and**
tie-offs; the project git ignores `rig/` entirely.

- ❌ Tie-offs and runtime data would not be committed with the project git
  — contradicts the stated requirement.
- ❌ The rig git fills with project-specific data, so a "reusable" rig
  repo is polluted the moment it runs — sharing it drags one project's
  output into the next.
- ✅ Minimal code change.

Rejected on both counts.

### Option C — Keep outputs in the rig, git-ignored; only source in rig git

Rig keeps `tie-offs/`, `state.json`, logs, queue on disk but git-ignored
in its own repo; project git ignores `rig/` entirely.

- ❌ Runtime data still lives inside the rig directory — the coupling the
  problem statement flags is only papered over by a `.gitignore`.
- ❌ No project-git audit trail (see Option B).
- ✅ Smaller move than Option A.

Rejected — Option A is only marginally more code and eliminates the
coupling structurally.

### Option D — Configurable runtime-root location (new rig-config key)

Add a key to `.workspace-agent-config.yaml` letting projects choose where
the runtime tree lives.

- ❌ Violates the zero-config, statically-derived path principle the
  project has moved toward repeatedly (0.22.0 flattening, static output
  paths, `derive_tieoff_path()`).
- ❌ Every skill and doc would have to say "configured location" instead
  of a concrete path — worse for the agents that read them.

Rejected; Option A's static namespacing covers the multi-rig case without
configuration.

### Sub-decisions within Option A

| Decision | Choice | Rationale / rejected alternative |
|---|---|---|
| Runtime artifact placement | All of `state.json`, `.rig-log`, `events/`, `{loom-id}/` (tie-offs, dispatch dirs, loom-logs) under `tie-offs/<rig-basename>/` | Project-specific runtime workflow data belongs to the project; committing them with Knot runs gives one clear audit history. *Alternative* — keep loom-logs/state in the rig, git-ignored — rejected: leaves project data inside the reusable rig and splits the audit trail across two places. |
| Tie-off namespacing | Uniform `tie-offs/<rig-basename>/…` | No collisions across rigs in one project; one rule for all rigs. *Alternative* — bare `tie-offs/{loom-id}/` + "loom ids unique per project" constraint — rejected: silently breaks dev/staging copies of the same rig. |
| Rig `.gitignore` | None — the rig directory is source-only | No runtime data to exclude. *Alternative* — write one anyway (defensive) — rejected: it implies runtime files may appear, which the layout now forbids. |
| Parent exclusion mechanism | Append `rig/` to parent `.gitignore` (marked, idempotent); warn with `git rm -r --cached` command when `rig/` is tracked | Shared with teammates, visible, idempotent. *Alternative* — `.git/info/exclude` (local-only, no tracked-file change) — rejected: not shared, invisible, and per-clone. |
| Where rig git init happens | In the Knot binary at startup (`run_startup`) | Guarantees the invariant for every rig Knot manages, including rigs created by plain `cargo run` in an empty dir. *Alternative* — `knot-init` skill only — rejected: skills don't always run; the invariant would be best-effort. |
| Auto vs manual file migration | Binary auto-migrates the old layout at startup (idempotent, logged); the parent untrack (`git rm --cached`) stays manual | Files the binary owns its layout for; dispatch directories and the queue must move or in-flight events are orphaned (the watcher doesn't re-scan old paths). Index/history surgery on the *project* repo is a user decision. |

## Existing Tests

| Test source | What it covers | Status |
|---|---|---|
| `src/domain/knot_file.rs` (unit) | `derive_tieoff_path()` / `derive_loom_log_path()` build `rig/tie-offs/{loom-id}/…` | ✅ Green — asserts the **old** paths; must move with the new derivation |
| `src/adapters/outbound/event_dispatcher.rs` (unit) | Event files written to `rig/tie-offs/{consumer-loom}/{EventId}/` | ✅ Green — asserts old base |
| `src/adapters/outbound/loom_log.rs` (unit) | Loom-log at `rig/tie-offs/{loom-id}/.loom-log` | ✅ Green — asserts old path |
| `src/application/usecases/loom/mod_watchers.rs` (unit) | Event watch dirs at `rig/tie-offs/{loom-id}/{EventId}/`, auto-creation | ✅ Green — asserts old base |
| `src/application/usecases/context_providers.rs` (unit) | Dispatch-directory fallback scan + fixture writer | ✅ Green — asserts old base |
| `src/application/usecases/process_strand.rs` (unit, incl. `git_versioning_tests`) | Tie-off path computation; `commit()` called on success only; failure tolerance | ✅ Green |
| `src/adapters/outbound/git_versioner.rs` (unit) | Commit in repo, graceful skip without repo, message format, body truncation, multiple commits | ✅ Green |
| `src/adapters/outbound/state_writer.rs`, `rig_log.rs`, `disk_event_queue.rs` (unit) | `state.json` at writer dir, `.rig-log` at rig dir, queue files at events dir | ✅ Green — constructor-based, paths follow the new runtime root |
| `tests/git_versioning.rs` | Versioner integration against real git in temp dirs | ✅ Green |
| `tests/tie_off.rs`, `tests/multi_loom.rs`, `tests/pipeline.rs`, `tests/agent_integration.rs`, `tests/smoke.rs`, `tests/rig_cli.rs`, `tests/rig_log.rs`, `tests/persistent_queue.rs` | End-to-end: rig startup, discovery, processing, on-disk artifacts, share command | ✅ Green — several assert `rig/…` artifact locations |
| `tests/helpers.rs`, `src/application/usecases/test_fixtures.rs` | Shared fixtures writing `rig/tie-offs/…` | ✅ Green — fixture paths must follow the new derivation |

## Test Gaps

- No test that the **runtime root is namespaced by rig basename**
  (default `rig` and a named `dev-rig` fixture).
- No test asserting the **new on-disk locations** after a full run
  (acceptance-level: startup → strand trigger → tie-off at
  `tie-offs/rig/{loom-id}/`, dispatch dir at
  `tie-offs/rig/{consumer-loom}/{EventId}/`, loom-log at
  `tie-offs/rig/{loom-id}/.loom-log`, state.json at
  `tie-offs/rig/state.json`, queue files at `tie-offs/rig/events/`).
- No test that the **rig directory contains no runtime files** after
  startup (no `state.json`, `events/`, `.rig-log`, `tie-offs/`).
- No test for **rig git init**: fresh init, idempotent re-run, git binary
  absent (graceful), no `.gitignore` written into the rig.
- No test for **parent exclusion**: `.gitignore` entry appended exactly
  once; warning (no edit) when `rig/` is tracked; no-op when parent is not
  a repo.
- No test for the **gitlink guard**: parent repo with a nested rig repo —
  after `commit()`, the rig is not staged as a gitlink or file.
- No test for **legacy migration**: old tree moves (tie-offs subtree,
  loom-logs, state.json, .rig-log, events/), rig left source-only,
  idempotent second run, partial-state (destination exists) keeps
  destination.
- No test for **startup ordering**: runtime root created before the first
  state write; migration before watcher registration (moved dispatch dirs
  watched at their new path).

## Phases

### Phase 0: Runtime-root derivation and consumers

Add the single source of truth — `derive_runtime_root(rig_dir)` in
`src/domain/knot_file.rs` — and re-root every consumer:
`derive_tieoff_path()` / `derive_loom_log_path()` (base → runtime root),
`process_strand.rs` (`compute_tie_off_path`), `event_dispatcher.rs`,
`loom_log.rs`, `mod_watchers.rs`, `context_providers.rs`,
`config_event_handler.rs`, plus the composition root wiring in
`server.rs` (state writer, rig log, event queue dirs) and
`tests/helpers.rs` / `test_fixtures.rs`. Unit tests in each file turn red
first, then green with the new expected paths. Named-rig (`dev-rig`)
fixtures added for the namespacing gap.

### Phase 1: Rig git initialisation

Extend `GitVersioningPort` with an idempotent rig-init method (e.g.
`ensure_rig_repo(rig_dir)`) and implement it in `git_versioner.rs`:
`git init` when `rig/.git` is absent (no `.gitignore` written), and — when
the project root is inside a git repo — append the marked `rig/` line to
the parent `.gitignore` or log the `git rm -r --cached` warning when
tracked. Call it from `run_startup()` after rig-dir creation, before
watcher registration. `MockGitVersioningPort` gains a no-op. All failure
modes (no git, no parent repo, already initialised) are non-fatal and
tested.

### Phase 2: Git versioner rig-awareness

`FileSystemGitVersioner::new(project_root, rig_dir)`; after `git add -A` in
`commit()`, run `git reset -q -- <rig-dir-relative>` so the rig can never
enter a project commit (gitlink or tracked leftovers). Integration tests
with real nested repos for both verified scenarios: fresh rig (gitlink
guard) and pre-tracked rig (stale-file guard).

### Phase 3: Legacy layout migration

Startup migration (before discovery/watchers, in `run_startup`): when the
old layout is detected, move `rig/tie-offs/` → `tie-offs/<rig-basename>/`
(subtree preserved, including `.loom-log` files), `rig/state.json` →
runtime root, `rig/.rig-log` → runtime root, `rig/events/` → runtime
root, leaving the rig source-only. Log a one-line `[startup] migrated …`
notice. Idempotent; destination-exists conflicts keep the destination and
warn. Unit tests cover the gap list above, including the ordering
guarantee that migration and runtime-root creation precede the first state
write and watcher registration (so moved dispatch dirs and the queue are
used at their new paths immediately).

### Phase 4: Skills, glossary, docs, changelog

- `knot-init` — **the "is Knot running" check moves** from
  `rig/state.json` to `tie-offs/<rig>/state.json` (default rig:
  `tie-offs/rig/state.json`); new "Rig Repository" section: Knot inits
  `rig/.git`, user commits manually, parent `.gitignore` auto-exclusion,
  one-time `git rm -r --cached rig/` for pre-existing projects;
  terminology lines updated (tie-off + runtime tree).
- `knot-glossary.md` — tie-off definition points at
  `tie-offs/<rig>/<loom-id>/`; the `tie-offs/` directory described as the
  rig's project-side runtime tree (tie-offs, logs, queue, state); the term
  "tie-off" (knot output document) itself is unchanged.
- `knot-manage` — two-repo review workflow (project git = output + runtime
  audit trail incl. logs/state snapshots; rig git = rig source); tie-off
  and loom-log paths updated; "no git repo" error table adjusted (rig git
  and project git are now independent).
- `knot-inspect`, `knot-analyst`, `knot-dispatch`, `knot-create`,
  `knot-design`, `knot-abstractions` — path references, quick-reference
  commands, and the architecture diagram updated (rig no longer holds any
  runtime data).
- `knot-update` — new **0.31.0** changelog entry: what moved, why,
  auto-migration note, manual untrack step, verification.
- `docs/` — `configuration/rig-structure.md` (directory tree + Git-Friendly
  section), `concepts.md`, `getting-started.md`, `troubleshooting.md`,
  `workflows/*.md`, `configuration/knots.md`, `release-notes.md`;
  `README.md`; `AGENTS.md` terminology line.
- Publish all updated skills to `~/.agents/skills/` and diff-verify
  (per AGENTS.md installation procedure).

### Phase 5: Migrate this repo's rig, bump, verify

Run the new binary against this repo's own rig (auto-migration of
`rig/tie-offs/` + runtime files → `tie-offs/rig/`, rig git init, parent
`.gitignore` entry), remove the legacy empty `rig/output/` dirs, then
manually `git rm -r --cached rig/` and commit the project side. Bump
version to **0.31.0** (breaking layout change), `cargo install --path .`,
and run a full smoke pass: fresh temp project → `cargo run` → rig git
exists, parent `.gitignore` updated, rig dir source-only, trigger a strand
→ tie-off/loom-log/state land under `tie-offs/rig/` and the Knot commit
touches them while `rig/` stays out of the commit.

## Notes

- **No PRD** — this is a cross-cutting architecture change to storage
  layout and git model, in the same class as 052 (flat tie-offs) and 056
  (event URI), which also skipped PRDs.
- **ADR at completion** — the rig/project repository split is a durable
  architectural decision; extract an ADR during plan completion
  (project-plan-completion).
- **Perpetually dirty project tree** — `state.json` is rewritten every 5
  seconds, so `git status` shows a modified
  `tie-offs/<rig>/state.json` whenever Knot has run recently. Knot commits
  fold it into the knot-run commit (a state snapshot per audit entry —
  intended). The user's own `git add -A` commits will also pick it up.
  Optional future optimisation (out of scope): skip the state write when
  the payload is unchanged — but that would break the `updated_at`
  freshness heartbeat that `knot-init` uses for the running check.
- **Event queue in git** — `events/*.json` files appear and disappear
  across commits as events are queued and popped. Net effect: pending work
  at any commit point (including crash-before-processing) is visible in
  project history. Some diff noise; accepted as part of the audit trail.
- **Watcher caveat** — `notify` does not rescan existing files when a
  watch starts. Migrated dispatch directories that already contain
  unprocessed event files must be touched after migration to re-trigger
  (documented in `knot-manage`'s file-watcher section).
- **`state.json`** `last_tie_off_path` values change automatically (derived
  at runtime); no schema change. The `rig_path` field continues to point
  at the rig directory (source), not the runtime root.
- **`knot share`** is unchanged (zips looms + profiles); the zip now equals
  exactly the rig git's tracked content, making git-remote sharing a
  natural alternative — worth a doc mention only.
- **Naming** — `tie-offs/` now holds runtime artifacts beyond tie-off
  documents. The directory name is kept (it is the established term in
  skills and docs, and the user-facing "tie-off folder"); glossary and
  docs describe it as the rig's project-side runtime tree.
- **Risk — rig basename edge**: a rig directory at the filesystem root has
  no parent; the existing fallback (`project_root = rig_dir`) applies and
  the runtime root falls back to being derived from the rig dir itself.
  Degenerate case, covered by a unit test.
