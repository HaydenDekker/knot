# Plan: Model Aliases — Rig-Level Model Registry for Profiles

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

The PRD's model-swap story ("I could set the profile LLM targets dynamically") is
currently satisfied only by editing every profile file that hard-codes
`provider` + `model`. This plan adds one level of indirection — a rig-level
alias registry — so swapping a model is a single-file edit that every
referencing profile picks up live, without a restart.

## Problem

Profiles hard-code `provider` and `model` in the frontmatter of
`rig/profiles/{name}.md`. When a model is replaced — a new model release, a
provider migration, a local-model swap — every profile file must be edited.
Models change far more often than knots do; the unit of change (model) is
misaligned with the unit of configuration (profile).

There is no rig-level indirection today. The only rig-level config file
(`.workspace-agent-config.yaml`) holds adapter wiring and is loaded **once at
startup** — the wrong lifetime for a registry that must support live swaps.
Profiles, by contrast, are read fresh from disk on every processing, which is
the lifetime the registry must follow.

## Target

- A new rig-level model registry at `rig/models.yml` maps **aliases** to
  `{provider, model}` pairs. It is read fresh at resolution time (per strand
  processing, per state write) — never cached at startup.
- Profiles gain an optional `model-ref` frontmatter field naming an alias.
  **`model-ref` takes highest priority over direct `provider`/`model`** —
  when both are present the alias wins and a parse warning is emitted.
- Direct `provider` + `model` profiles remain valid indefinitely — non-breaking,
  no forced migration.
- Swapping a model = editing one line in `rig/models.yml`; the next strand
  processed by any knot referencing the alias uses the new model. No restart.
- `run_startup` auto-creates `rig/models.yml` (commented template) when
  missing — idempotent, never overwrites an existing file.
- `tie-offs/<rig>/state.json` profile entries show `model-ref` plus the
  **resolved** `provider`/`model` (null when the alias is unresolvable).
- The `knot-init` skill seeds a `default:` alias in `rig/models.yml` and the
  default profile uses `model-ref: default`.
- Skills updated: knot-update (changelog entry), knot-create (profile format +
  swap workflow), knot-inspect (profile listing), knot-init (alias seeding).

## Format

### `rig/models.yml`

```yaml
models:
  fast:
    provider: openai
    model: gpt-4o
  frontier:
    provider: anthropic
    model: claude-sonnet-4-20250514
```

- Top-level `models` map; alias → `{provider, model}`. Both `provider` and
  `model` are required, non-empty per alias.
- Alias names: any non-empty string (no slug enforcement — aliases are YAML
  keys, not file names). Aliases are semantic (`fast`, `frontier`); two
  aliases may target the same model (A/B swapping is a feature).
- File missing → empty registry (direct-spec profiles unaffected; alias
  profiles fail to resolve).
- Malformed YAML → warning, treated as empty registry.

### Profile frontmatter

```yaml
---
name: fast
model-ref: fast
tools:
  - read
---
```

| `model-ref` | `provider` + `model` | Behaviour |
|---|---|---|
| set | absent | Resolved via registry at processing time |
| set | set | **Alias wins** — direct values ignored, parse warning |
| absent | set | Legacy direct spec — unchanged |
| absent | absent | Parse error (no model defined) |

Unknown alias → knot run fails with `ModelRefNotFound` (loom-log entry +
`last_error` in state), message pointing at `rig/models.yml`.

## Existing Tests

| Test location | What it covers | Status |
|---|---|---|
| `src/domain/value_objects.rs` (`agent_profile_*`) | `AgentProfile` construction, validation, serialisation, `resolve_for_knot` field mapping | ✅ Green — defines current behaviour |
| `src/domain/knot_file.rs` (`parse_profile_*`) | Profile frontmatter parsing: name/provider/model/tools/timeout + error cases | ✅ Green — defines current behaviour |
| `src/adapters/outbound/profile_repo.rs` | Profile file get/list from disk, malformed-file skipping | ✅ Green |
| `src/application/usecases/process_strand.rs` (`resolve_agent_config_*`) | Profile→`AgentConfig` mapping, `ProfileNotFound`, dynamic profile pickup, CLI arg construction | ✅ Green — defines current behaviour |
| `src/application/usecases/write_state.rs` | State snapshot incl. profiles array | ✅ Green |
| `tests/profile_timeout.rs` | Integration: per-profile timeout via `ProcessStrandBuilder` | ✅ Green |
| `tests/agent_integration.rs` | Integration: agent invocation, tie-offs, state, failure paths | ✅ Green |
| `tests/helpers.rs` (`ProcessStrandBuilder`) | Shared integration builder with `with_profile()` | ✅ Green |

## Test Gaps

- No `ModelRegistry` value object exists — no YAML parse/resolve tests.
- No tests for `model-ref` parsing, alias-over-direct precedence, or the
  "no model spec at all" validation.
- No test that resolution picks up a **changed** registry between two runs
  (the live-swap guarantee).
- No test for the `ModelRefNotFound` failure path (loom-log entry, no tie-off
  write, state error).
- No state-schema test for `model-ref` + resolved values (and nulls when
  unresolvable).
- No acceptance-level integration test exercising the full pipeline with a
  real `models.yml` on disk.
- No test that `run_startup` creates `rig/models.yml` when missing and never
  overwrites it.

## Phases

### Phase 0: Domain — `ModelRegistry` and `AgentProfile.model_ref`

Failing tests first, then implementation:

- `ModelRegistry`: YAML parse (valid, empty file, missing `models` key,
  malformed, empty provider/model rejected), `resolve()` hit/miss, empty
  registry.
- `AgentProfile`: new `model_ref: Option<String>` field (serde name
  `model-ref`); validation — model-ref-only profile is valid; no model spec at
  all is an error; empty `model-ref` is an error.
- `resolve_for_knot(profile, knot, registry)` returns `Result`: alias resolved
  from registry; alias takes priority over direct values; unknown alias →
  `ModelRefNotFound(alias)`; direct-spec path unchanged.

Existing unit tests updated to the new signature in the same phase. A domain
error variant for unknown aliases is added (keep errors domain-owned).

### Phase 1: Port + Adapter — `ModelRegistryPort` / `FileSystemModelRegistry`

- `ModelRegistryPort` trait in `src/application/ports.rs`:
  `load() -> Result<ModelRegistry, PortError>` — fresh read per call.
- `FileSystemModelRegistry` adapter in `src/adapters/outbound/` reading
  `{rig_dir}/models.yml`. Adapter tests with `tempfile`: valid file, missing
  file → empty registry, malformed → empty registry + warning.

### Phase 2: Application — Resolution in `ProcessStrand`

- `ProcessStrand` gains a `model_registry: Arc<dyn ModelRegistryPort>`
  dependency; `tests/helpers.rs` `ProcessStrandBuilder` gains
  `with_model_registry()` (default: in-memory mock).
- `execute()` loads the registry per run, resolves the profile, and maps the
  domain error to `PortError::ModelRefNotFound`; the failure path records a
  loom-log entry and leaves the tie-off unchanged (mirrors `ProfileNotFound`).
- Application tests with a mock registry port: resolved alias reaches the CLI
  args (`--model <registry model>`); unknown alias fails with
  `ModelRefNotFound`; empty registry + `model-ref` fails; direct-spec profiles
  unaffected; **live-swap test** — registry content changes between two
  `execute()` calls and the second run uses the new model.

### Phase 3: State Visibility — `RigStateProfile` with Alias + Resolved Model

- `RigStateProfile` gains `model_ref: Option<String>`; `provider`/`model`
  become `Option<String>` (null when the alias is unresolvable).
- `write_state` loads the registry and emits resolved values.
- Tests: alias profile → `model-ref` + resolved provider/model in state;
  direct profile → `model_ref` null; unknown alias → provider/model null +
  warning logged.

### Phase 4: Composition, Startup Auto-Create, Integration (Acceptance)

- `run_startup` creates `rig/models.yml` (commented template, matching the
  `.workspace-agent-config.yaml` pattern) when missing; never overwrites.
- `build_app_context` wires `FileSystemModelRegistry` into the strand
  pipeline and the state writer.
- Acceptance-level integration tests (real `FileSystemModelRegistry` +
  `tempfile` rig tree, fake CLI runner):
  1. Full flow: `models.yml` + `model-ref` profile → tie-off written, runner
     received the resolved `--model`.
  2. Live swap: rewrite `models.yml`, trigger a second strand → second run
     uses the new model, no restart.
  3. Unknown alias → knot fails; error visible in state `last_error` and
     loom-log.
  4. Regression: legacy direct-spec profile with no `models.yml` still runs.
  5. `run_startup` creates the file; a second run does not overwrite it.

### Phase 5: Skills, Docs, Version

- **knot-update**: changelog entry (0.32.0) — new `model-ref` field,
  `rig/models.yml` registry, precedence table, migration steps (collect
  distinct provider/model pairs → define aliases → replace frontmatter lines),
  search patterns.
- **knot-create**: profile section — `model-ref` field, registry format,
  precedence table, "swap a model behind an alias" workflow.
- **knot-inspect**: profile listing shows alias + resolved model (null =
  unresolvable alias).
- **knot-init**: seed a `default:` alias in `rig/models.yml` from
  `~/.pi/agent/models.json`; default profile uses `model-ref: default`.
- Bump skill versions, publish updated skills to `~/.agents/skills/`, verify
  with diff (per AGENTS.md).
- Version bump to 0.32.0 and `cargo install --path .` handled at plan
  completion.

## Notes

- **Lifetime is the core constraint.** `.workspace-agent-config.yaml` is
  loaded once at startup (adapter wiring — restart-required by nature). The
  registry must follow the profile lifetime instead: fresh read per
  resolution. Do not cache.
- **Priority decision** (from design review): `model-ref` takes highest
  priority over direct `provider`/`model` rather than erroring on ambiguity.
  The parse warning keeps drift visible and encourages cleanup of redundant
  direct fields.
- **File extension:** `models.yml` (not `.yaml`), per design decision. The
  existing dotfile keeps `.yaml` — do not rename it in this plan.
- **Alias naming:** no slug validation; any non-empty key.
- **Registry scope:** alias → `{provider, model}` only. No per-model timeout
  or args — timeout is per-profile; keep the registry minimal.
- **State schema:** `provider`/`model` becoming `Option<String>` is additive
  for consumers reading them as strings; the knot-inspect skill treats null as
  "unresolvable alias".
- The stale doc comment on `AgentProfile` ("knots … may override individual
  fields (model, tools) inline") is wrong — knots have no model override.
  Correct the comment while touching this file.
- Plan 068 (rig-repo-separation) moved the runtime tree to
  `tie-offs/<rig>/` — all paths in this plan use the post-0.31 layout.
