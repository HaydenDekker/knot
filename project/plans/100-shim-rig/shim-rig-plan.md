# Plan: Shim Rig — Externalised Workflow Rig with Local Project Shim

## Related PRD

This plan contributes to [Shim Rig — Externalised Workflow Rig](../../prds/prd-shim-rig.md).

It delivers the PRD in full: the `external-rig` configuration key, the
shim-rig load model (external looms/knots/profiles, local configuration,
disjoint sides with warn-and-ignore), the hard-fail contract for a missing
external rig, hot-reload over the external rig, and provenance in
`state.json` and the service log.

## Problem

Every project that runs a stable workflow keeps its own local copy of the
rig directory, so a workflow shared across N projects is maintained in N
copies. The rig was always designed to be a standalone reusable unit ("…
versioned in its own git repository" — glossary), and `knot share <rig>`
ships it as a zip, but a snapshot is still a copy. Projects need a **live
reference** to a shared external rig, while keeping what is genuinely
project-local: rig-level configuration (`models.yml`,
`.workspace-agent-config.yaml`, instruction overrides) and the runtime tree.

Settled decisions (from the design discussion, 2026-10-27):

1. **Local path only** — `external-rig` names a local directory (absolute,
   `~/...`, or project-root-relative). The service never fetches; a git
   checkout is a valid target, but updating it is a user/script concern.
2. **Whole-rig import** — a shim rig imports the external rig whole. No
   per-loom or per-profile selection.
3. **No overriding, disjoint sides** — when `external-rig` is set, looms,
   knots, and profiles come from the external rig *or* the local rig, never
   merged. Local looms/profiles in a shim rig are **ignored** with a
   startup warning listing each ignored item.
4. **No local-loom extension** — a local knot cannot join an external loom.
5. **Hard error on missing external rig** — nothing starts; the diagnostic
   names the path and the declaring config file; non-zero exit.
6. **Terminology** — "shim rig" is the term of art; added to the glossary.

## Target

- `rig/.workspace-agent-config.yaml` accepts `external-rig: <path>`. A rig
  with the key is a **shim rig**; its effective workflow (looms, knot
  files, profiles) is served from the resolved external rig directory.
- Startup, with the key set:
  - external path missing / not a directory / unreadable → hard startup
    error: clear `[KNOT][CONFIG]` diagnostic on stderr, non-zero exit,
    nothing started;
  - local `-loom/` directories or `profiles/*.md` present → warning listing
    every ignored local item;
  - success → `[KNOT][CONFIG]` line records the resolved external path.
- Everything downstream of rig loading (loom discovery, knot parsing,
  profile resolution, the file watcher, the config pipeline, hot reload)
  operates on the **effective** rig directory — so external-rig edits are
  picked up live, exactly like today's local-rig edits.
- Rig-level configuration always resolves from the **local** shim rig:
  `models.yml` (alias registry), `agent-adapter`, instruction-file links,
  `knot-template/` overrides. If the external rig carries its own
  `models.yml` or `.workspace-agent-config.yaml`, they are not loaded.
- The runtime tree is unchanged: `tie-offs/<rig-basename>/` under the
  consuming project, keyed by the **local** rig directory's basename.
- `state.json` gains an `external_rig` provenance field (resolved absolute
  path; absent for non-shim rigs).
- A rig without the key behaves byte-for-byte as today.

## Existing Tests

| Test | What it covers | Status |
|------|---------------|--------|
| `tests/rig_discovery.rs` | `discover_rigs` — `*-rig` scan, explicit name, multiple | ✅ Green — defines current discovery |
| `tests/discovery.rs` | Loom/knot discovery from a rig dir + config-pipeline reload | ✅ Green — currently always local |
| `tests/composition.rs` | `build_app_context` wiring (store, ports, rig config) | ✅ Green — no external resolution |
| `tests/model_aliases.rs` | `models.yml` alias registry: parse, resolution, override | ✅ Green — stays local-side |
| `tests/step.rs`, `tests/rig_cli.rs` | `knot step` and CLI surface incl. `--rig` | ✅ Green — must stay unchanged |
| `tests/smoke.rs` + `tests/fixtures/` | Mock-CLI harness for end-to-end runs | ✅ Green — the acceptance vehicle |

## Test Gaps

- No concept of an *effective* rig directory — nothing exercises
  "config says external, rig loads from there".
- No startup failure path for an unresolvable external path (hard error,
  non-zero exit, diagnostic content).
- No warn-and-ignore behaviour for local looms/profiles in a shim rig.
- No hot-reload test across a local/external boundary (edit in external
  rig, observe config reload in the project).
- No provenance assertion in `state.json` / startup config lines.
- No acceptance-level test of a full shim run (strand → external knot →
  tie-off in the *project's* runtime tree).

## Phases

### Phase 0: Configuration surface and path-resolution contract

Extend the rig agent config with `external-rig: Option<String>` and add a
pure domain resolver (absolute / `~/...` / project-root-relative → resolved
path; validation result: resolved / missing / not-a-directory /
unreadable / config error). Unit tests define the resolution contract and
the malformed-input behaviour (unknown keys, empty value, non-string).
Files: `src/domain/` (config value object + resolver),
`src/adapters/outbound/` or `src/server.rs` (`load_rig_config`), unit tests.
Pane: the resolver's unit tests green.

### Phase 1: Startup resolution and hard-fail

The startup path (service + `knot step` where it loads a rig) resolves the
effective rig directory from the local config; a failed resolution logs the
`[KNOT][CONFIG]` diagnostic and exits non-zero before anything is wired.
Success logs the resolved external path. Verified with the mock-CLI harness:
a shim project with a valid external rig starts; with a missing path it
refuses to start with the expected diagnostic and exit code; a rig without
the key starts exactly as before.
Files: `src/main.rs` / `src/server.rs` (startup + `run_startup`), harness
tests. Pane: the startup accept/reject harness tests green.

### Phase 2: Serve looms, knots, and profiles from the effective rig

Loom discovery, knot parsing, and profile loading point at the effective
rig directory; `models.yml` and the rest of the local config stay local.
Local `-loom/` dirs and `profiles/*.md` in a shim rig produce the
list-each-item warning; an external rig's own `models.yml` /
`.workspace-agent-config.yaml` is not loaded. Unit + composition tests.
Files: `src/application/usecases/loom/`, profile-loading call sites,
`src/server.rs` composition, `tests/discovery.rs` / `tests/composition.rs`
fixtures. Pane: the composition tests green (external rig served, locals
ignored-and-listed).

### Phase 3: Hot-reload over the external rig

The file watcher and config pipeline watch the **effective** rig directory,
so an edit/add/remove in the external rig reloads live in the project —
same debounce/reload semantics as today's local edits. Harness tests: edit
an external knot file (reload), add an external loom (discovered live),
remove one (retired). Files: `src/server.rs` (watcher wiring), harness
tests. Pane: the live-reload harness tests green.

### Phase 4: Provenance

`state.json` carries `external_rig` (resolved absolute path; absent for
non-shim rigs — no diff for existing rigs) and the startup config log
records it. State-writer and state-read tests assert the field's presence
and shape, and that a plain rig's state is unchanged. Files:
`src/application/usecases/write_state.rs`, state value types,
state-related tests. Pane: the state assertions green.

### Phase 5: Acceptance, `knot share`/`knot step` compatibility, docs and skills

Acceptance-level test: a shim project runs a full strand → external knot →
tie-off, with the commit, `tie-offs.log` section, queue lifecycle, and
`state.json` all in the **project's** runtime tree; `knot step` against the
shim rig and `knot share` against the external rig remain correct (shared
tests). Then the durable surfaces: glossary term **Shim Rig**, the
knot-update changelog entry (0.x → next release, migration: none — the key
is additive), and the skill updates that must keep telling the truth
(`knot-create`, `knot-inspect`, `knot-start`, `knot-manage`, `knot-init` —
shim-aware notes only where the rig location matters). Files: harness
acceptance test, `project/plans/100-shim-rig/`, `.agents/skills/knot-init/
knot-glossary.md`, `.agents/skills/knot-update/SKILL.md`, affected
sub-skills. Pane: the end-to-end shim acceptance test green + docs/skills
consistent.

## Notes

- **Rig identity**: the runtime tree and the `--rig` CLI target stay keyed
  by the *local* rig directory basename — two projects shimming the same
  external rig have disjoint, per-project runtime trees with zero
  configuration. The external path is provenance, not identity.
- **`knot share` on a shim**: decided in Phase 0 — expectation: it shares
  what the rig serves (the effective/external rig contents), with a CLI
  note when the rig is a shim; alternatively explicit out-of-scope (the
  external rig is shareable directly from its own directory). The Phase 5
  compatibility tests pin whichever decision Phase 0 records.
- **Trust model**: an external rig is prompt text executed from another
  repository — same trust posture as pulled skills. Provenance in
  `state.json` is the mitigation; no lock/pinning in this plan (the git
  ref form and resolved-SHA recording come with it, later).
- **`run_startup` file creation**: today `run_startup` seeds
  `profiles/` and the config file in the local rig dir. For a shim rig it
  must seed only the local-side files (config, `models.yml` if missing) and
  must **not** create looms/profiles in the external rig — Phase 1 tests
  pin this.
- **knot-lint**: a later check (out of this plan's scope) can flag a shim
  rig that carries ignored local looms/profiles; the startup warning is
  the interim surface.
