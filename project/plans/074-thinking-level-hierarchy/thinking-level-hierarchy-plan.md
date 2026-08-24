# Plan: Thinking Level — Alias Default with Profile Override

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

The PRD's "set the profile LLM targets dynamically" story is extended by plan
[069 Model Aliases](../069-model-aliases/model-aliases-plan.md) (which model an
agent uses). This plan adds the companion dial: **how hard that model thinks**.
Today every pi invocation runs at whatever thinking level pi's own settings
default to — a rig cannot express "this agent thinks deeply, that one does
not". The user story addressed: *as a rig owner I want to set reasoning
effort per model alias (default) and per agent profile (override), so the
thinking budget follows the role, not the machine.*

## Problem

pi exposes reasoning effort as a **thinking level** (`off | minimal | low |
medium | high | xhigh`) — its CLI takes `--thinking <level>` (all modes,
including the `-p` print mode Knot invokes) and maps the level internally to
provider-specific parameters (OpenAI `reasoning_effort`, OpenRouter
`reasoning: { effort }`, DeepSeek `thinking: { type }`, …). pi clamps the
level to the model's capabilities (non-reasoning models run `off`), so
passing a level is always safe.

Knot's invocation chain — profile → `AgentProfile` →
`resolve_for_knot()` → `AgentConfig` → `build_cli_args()` → `pi` subprocess
— carries no thinking-level field at all. Consequences:

- No way to make a "deep analyst" profile think harder or a "fast triage"
  profile think less without editing pi's global settings (per-machine,
  not per-rig, not versioned).
- The rig-level model registry (`rig/models.yml`, plan 069) is the natural
  home for model-behaviour defaults, but it only carries `provider`/`model`.

## Target

- A new domain value object `ThinkingLevel` (`off | minimal | low | medium |
  high | xhigh`), validated at parse time so typos fail at file parse, not
  at spawn.
- `rig/models.yml` alias entries gain an **optional** `thinking-level` —
  the default for every profile that resolves that alias.
- Profile frontmatter gains an **optional** `thinking-level` — when present
  it **takes precedence over the alias default** (profile is more specific
  than alias). Direct-spec profiles (`provider` + `model`) use their own
  `thinking-level` only — the registry is not consulted, mirroring how
  `provider`/`model` are sourced.
- Resolution rule (effective level):
  `model-ref` profile → `profile.thinking_level.or(alias.thinking_level)`;
  direct-spec profile → `profile.thinking_level`.
- The effective level is emitted as `--thinking <level>` in
  `build_cli_args()` for **every** effective value, including `off`
  (explicit off overrides pi's settings default). Omission emits **no
  flag** (pi's own default applies). Silence ≠ forced off — that asymmetry
  is intentional.
- `tie-offs/<rig>/state.json` profile entries gain an optional
  `thinking-level` showing the **effective** level (profile override or
  alias default), omitted when neither sets one — observable parity with
  `timeout`.
- Non-breaking: all new fields are optional with serde defaults. Existing
  `models.yml` files and profile files parse unchanged; state.json
  consumers see a new key only when a level is set.
- The auto-created `models.yml` template (startup) documents the new key.
- Skills updated: `knot-create` (profile frontmatter table + models.yml
  section), `knot-inspect` (profile listing), `knot-init` (models.yml
  seeding note, if it carries a template), `knot-update` (changelog entry
  at completion).

### Format

`rig/models.yml`:

```yaml
models:
  fast:
    provider: openai
    model: gpt-4o
    thinking-level: low        # optional default for this alias
  frontier:
    provider: anthropic
    model: claude-sonnet-4-20250514
    thinking-level: high
```

Profile frontmatter (`rig/profiles/{name}.md`):

```yaml
---
name: analyst
model-ref: frontier            # alias default: high
thinking-level: xhigh          # profile override wins
tools:
  - read
---
```

- Field name in both files: `thinking-level` (kebab-case, matching pi's
  CLI terminology and its `--thinking` flag).
- Allowed values: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`.
  Anything else is a parse error in whichever file it appears in.

## Architecture (hexagonal)

| Layer | What changes |
|-------|--------------|
| Domain | `ThinkingLevel` enum (new); `ModelRef.thinking_level`; `ModelRegistry::from_yaml` validation + `ModelRegistryError::InvalidThinkingLevel { alias, value }`; `AgentProfile.thinking_level` + `with_thinking_level` builder; `parse_agent_profile` frontmatter field + `AgentProfileError::InvalidThinkingLevel(String)`; `AgentConfig.thinking_level`; `AgentProfile::resolve_for_knot` hierarchy rule; `AgentConfig::build_cli_args` `--thinking` emission |
| Application | `write_state.rs` — map the **effective** level into the state snapshot. `process_strand.rs` unchanged (already calls `resolve_for_knot`) |
| Ports | Unchanged — `ModelRegistryPort` already returns the whole registry (alias default travels with it); `AgentRunnerPort` unchanged |
| Inbound adapter | HTTP state endpoint surfaces the new state.json key automatically — no route changes |
| Outbound adapter | `pi_stdio.rs` / `pi_json.rs` **unchanged** — both pass `build_cli_args()` through untouched; `model_registry.rs` (file load) unchanged |

CLI arg order produced: `["-p", "--model", <model>, "--thinking", <level>?,
"--tools", <list>?, <extra_args>…]` — `--thinking` sits with the model
options, before `--tools`.

## Existing Tests

| Test source | What it covers | Status |
|-------------|----------------|--------|
| `src/domain/value_objects.rs` (unit) | `build_cli_args` base/tools/extra-args shape; `resolve_for_knot` alias resolution from registry; `ModelRegistry::from_yaml` valid/malformed/missing-field errors | ✅ Green — defines current arg shape and resolution |
| `src/domain/knot_file.rs` (unit) | Profile frontmatter parsing: `model-ref`, `tools`, `timeout`, alias-over-direct precedence | ✅ Green — defines current profile format |
| `tests/model_aliases.rs` (integration) | End-to-end alias resolution through `ProcessStrand` with a real registry file; asserts `build_cli_args()` output of the resolved config | ✅ Green — the acceptance pattern to extend |
| `src/adapters/pi_stdio.rs` (adapter unit) | Mock-CLI argv pass-through (`--name`, stdin prompt) via `KNOT_TEST_CLI_PATH` | ✅ Green — the argv-capture pattern to extend |
| `src/adapters/pi_json.rs` (adapter unit) | `--mode json` appended to base args | ✅ Green |
| `src/application/usecases/write_state.rs` (unit) | state.json profiles: `model-ref` + resolved `provider`/`model`, unresolvable-alias `null`s | ✅ Green — defines current state shape |
| `tests/profile_timeout.rs` (integration) | Reference pattern: a profile-level knob (`timeout`) flowing through mocked ports | ✅ Green |

## Test Gaps

- No `ThinkingLevel` type or field exists anywhere — all behaviour is new;
  nothing green today encodes a thinking level.
- No test that a registry or profile `thinking-level` value reaches the
  spawned `pi` argv (acceptance gap — must be closed through the real
  adapter, mock-CLI argv capture).
- No test for the precedence hierarchy (profile > alias, four-case
  matrix).
- No test for invalid-value rejection in either file (parse-time error with
  alias/profile context in the message).
- No state.json test for the effective `thinking-level` on profile entries.
- No `pi_json` argv pass-through test for a model-scoped flag (existing
  coverage is `--name` in `pi_stdio`); `--thinking` should be asserted in
  both runners since both build from `build_cli_args()`.

## Phases

### Phase 1: ThinkingLevel value object + registry parsing

Failing tests first: `ThinkingLevel` serde round-trips against YAML tokens
(`off` … `high`, and `xhigh` — note the enum variant needs an explicit
`#[serde(rename = "xhigh")]`; blanket kebab-case would yield `x-high`),
`Display` yields the exact CLI token; `ModelRegistry::from_yaml` accepts
`thinking-level` on an alias, leaves `None` when absent, and rejects an
invalid value with `InvalidThinkingLevel` naming the alias and the bad
value. Implement the enum, the `ModelRef` field, and the
string→enum conversion in `from_yaml` (raw entry stays `Option<String>` so
the error is precise, matching the existing provider/model pattern).

### Phase 2: Profile parsing, config fields, CLI emission

Failing tests first: `parse_agent_profile` accepts `thinking-level` in
frontmatter, defaults to `None`, rejects invalid values with
`AgentProfileError::InvalidThinkingLevel`; `build_cli_args()` omits
`--thinking` when unset and emits `--thinking <level>` between `--model
<model>` and `--tools` when set (including `off`). Implement the
frontmatter field, `with_thinking_level` builder (fluent, like
`with_timeout`), the `AgentProfile`/`AgentConfig` fields, and the
`build_cli_args` emission.

### Phase 3: Hierarchy resolution in resolve_for_knot

Failing tests first — the full matrix on `resolve_for_knot`:
(profile, alias) ∈ {None, None} → None; {Some, None} → profile;
{None, Some} → alias; {Some, Some} → profile (override). Plus: direct-spec
profiles resolve their own level with the registry unconsulted; an alias
without a level still lets the profile's level through. Implement the
`profile.or(alias)` rule in the `model-ref` branch.

### Phase 4: State visibility

Failing tests first: a written state snapshot shows the effective
`thinking-level` on profile entries in all three shapes — profile-only,
alias-default, and omitted when neither sets one (key absent, not null,
matching the `timeout` convention). Implement the `RigStateProfile` field
(renamed `thinking-level`, skip-if-none) and the effective-level mapping in
`write_state.rs`.

### Phase 5: Acceptance through the adapters + rig docs

Acceptance-level tests first (full flow through the adapter layer): a
mock CLI script (existing `KNOT_TEST_CLI_PATH` pattern) that records its
argv; resolve a profile with `model-ref` against a real `models.yml`
containing an alias default, with the profile overriding it; run
`PiStdioAgentRunner` **and** `PiJsonAgentRunner`; assert the spawned argv
contains `--thinking <effective-level>` in both. Then: update the
auto-created `models.yml` template comment (`src/server.rs`), update skills
(`knot-create` frontmatter table + models.yml section + example,
`knot-inspect` profile listing, `knot-init` seeding note where applicable,
`knot-update` changelog entry), and run the full suite — no regressions in
existing arg-shape or state-shape tests.

## Notes

- **Naming**: `thinking-level` (pi's terminology, 1:1 with `--thinking`)
  rather than `reasoning-effort` (the raw OpenAI wire parameter). pi's
  level is provider-agnostic and mapped per provider internally; our field
  should not assume OpenAI.
- **No capability validation needed**: pi clamps the level to the model's
  supported levels (non-reasoning models always run `off`; `xhigh` is
  honoured only where supported). A `high` on a non-reasoning alias is
  harmless — validation is lexical only.
- **`off` vs omission** (see Target): explicit `off` emits the flag;
  omission does not. Document this in the skill docs so users don't expect
  "absent = off".
- **Backward compatibility**: serde `#[serde(default,
  skip_serializing_if = "Option::is_none")]` on every new field — old
  profile/registry files and state.json readers are unaffected.
- **Version**: MINOR bump 0.35.0 → 0.36.0 at plan completion (new
  non-breaking feature; `knot-update` changelog entry records the field
  additions).
