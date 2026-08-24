# Design: Thinking Level — Alias Default with Profile Override

**Type:** Subsystem reference
**Subsystem:** profile/registry resolution and CLI emission
(`ThinkingLevel`, `ModelRegistry`, `AgentProfile`, `AgentConfig` in
`src/domain/`; `write_state.rs`; the `pi` argv built by
`AgentConfig::build_cli_args`)

## What It Is

A rig can now express **how hard a model thinks** — pi's reasoning
effort, a *thinking level* — at two levels of the profile→model
hierarchy:

1. **Alias default** — `rig/models.yml` alias entries may carry an
   optional `thinking-level`; it applies to every profile that resolves
   the alias.
2. **Profile override** — profile frontmatter may carry an optional
   `thinking-level`; when present it **takes precedence over the alias
   default** (the profile is more specific than the alias).

The resolved **effective** level is emitted as `--thinking <level>` on
the pi invocation for **every** effective value — including an explicit
`off`. Omitting the field everywhere emits **no flag**: pi's own
settings default applies. The asymmetry is intentional — silence is not
a forced off.

Allowed values: `off | minimal | low | medium | high | xhigh` (pi's
tokens, 1:1 with its `--thinking` flag). Anything else is rejected at
file-parse time — never at spawn.

```yaml
# rig/models.yml
models:
  frontier:
    provider: anthropic
    model: claude-sonnet-4-20250514
    thinking-level: high        # alias default
```

```yaml
# rig/profiles/analyst.md
---
name: analyst
model-ref: frontier             # alias default: high
thinking-level: xhigh           # profile override wins
---
```

## Why

- **The thinking budget followed the machine, not the role.** Every pi
  invocation ran at whatever pi's global settings defaulted to — a
  per-machine, non-versioned dial. A "deep analyst" profile and a "fast
  triage" profile could not be distinguished without editing pi's
  settings on each workstation.
- **The registry was the natural home for model-behaviour defaults.**
  `rig/models.yml` (plan 069) already carries the model-behaviour
  identity (`provider`/`model`) per alias; `thinking-level` extends the
  same entry. The profile is the natural override point — it already
  wins over the alias for model selection.

## Resolution Hierarchy

`AgentProfile::resolve_for_knot(&knot, &registry)` computes the
effective level at the same site as model selection (a single match over
`model_ref`, yielding `(provider, model, thinking_level)`):

| Profile `thinking-level` | Alias `thinking-level` | Effective |
|---|---|---|
| `None` | `None` | `None` — no `--thinking` flag |
| `Some(p)` | `None` | `p` |
| `None` | `Some(a)` | `a` |
| `Some(p)` | `Some(a)` | `p` — profile overrides alias |

- **`model-ref` branch:** `self.thinking_level.or(alias.thinking_level)`.
  `ThinkingLevel: Copy`, so this is by-value.
- **Direct-spec branch** (`provider` + `model`): the profile's own
  `thinking_level` only — the registry is **not consulted**, mirroring
  how `provider`/`model` are sourced. A same-named alias carrying a
  different level has no effect.
- **Unresolvable alias** (state-snapshot path only): the state writer
  degrades to the profile's own level alongside the existing `null`
  provider/model; the processing path still fails with
  `ModelRefNotFound`.

## CLI Emission

`AgentConfig::build_cli_args()` — the single argv builder both pi
runners pass through untouched (`pi_stdio.rs`, `pi_json.rs` are
unchanged by this plan):

```
["-p", "--model", <model>, "--thinking", <level>?, "--tools", <list>?, <extra_args>…]
```

- `--thinking <level>` sits with the model options — after `--model`,
  before `--tools`.
- Emitted for every `Some(level)`, **including `Off`** — `--thinking
  off` forces off and overrides pi's settings default.
- `None` → no flag at all; pi's own settings default applies.

### Why lexical validation only

pi clamps a level to the model's capabilities (non-reasoning models run
`off`; `xhigh` is honoured only where supported). A `high` on a
non-reasoning alias is harmless, so Knot does not validate levels
against model capability — validation is exactly the six-token
vocabulary, and the error fires at file parse, not at spawn.

### Why `thinking-level`, not `reasoning-effort`

`reasoning_effort` is the raw OpenAI wire parameter. pi's level is
provider-agnostic (it maps per provider internally: OpenAI
`reasoning_effort`, OpenRouter `reasoning: { effort }`, DeepSeek
`thinking: { type }`, …) — the field should not assume OpenAI, and
1:1 naming with `--thinking` keeps the config readable against pi's
own CLI.

## Parsing and Errors

| File | Field | Invalid value |
|---|---|---|
| `rig/models.yml` | alias `thinking-level` | `ModelRegistryError::InvalidThinkingLevel { alias, value }` — the registry loader degrades to a **warning + empty registry** (never blocks processing), matching the existing malformed-entry behaviour |
| `rig/profiles/*.md` | frontmatter `thinking-level` | `AgentProfileError::InvalidThinkingLevel(String)` — a **hard parse error** naming the offending value |

`ThinkingLevel::parse(token) -> Option<ThinkingLevel>` is the single
lexical validator (exact, case-sensitive, no trim); `Display` is its
inverse — the exact CLI token. The registry's raw entry stays
`Option<String>` so the error carries the offending value (same pattern
as `provider`/`model`); the profile parses the frontmatter string the
same way.

**Serde note:** `rename_all = "kebab-case"` would serialise `XHigh` as
`x-high`; the variant carries an explicit `#[serde(rename = "xhigh")]`.

## State Visibility

`RigStateProfile.thinking_level` (serde `thinking-level`,
`skip_serializing_if = "Option::is_none"`) shows the **effective**
level — profile override or alias default. The key is **omitted**
(never `null`) when neither sets one, matching the `timeout`
convention; existing `state.json` consumers see a new key only when a
level is set. The mapping lives in the same `match` in
`write_state.rs` that resolves `model_ref`/`provider`/`model` (a
4-tuple), so the state snapshot and the actual invocation stay in
lockstep.

## Backward Compatibility

Every new field is optional with serde `default` + skip-if-none:

- Existing `models.yml` files and profile files parse unchanged.
- Pre-0.36.0 binaries ignore the unknown YAML keys (no
  `deny_unknown_fields`) — setting a level on an old binary is inert,
  not an error.
- Without a `thinking-level` anywhere, no `--thinking` flag is emitted
  and pi's settings default applies — exactly the pre-0.36.0 behaviour.
- `state.json` readers: new key only when set; omitted (not `null`)
  otherwise.

## Components

| Piece | Location | Role |
|---|---|---|
| `ThinkingLevel` (+ `parse`, `Display`) | `src/domain/value_objects.rs` | Six-token enum; `Copy`; lexical validation + exact CLI token |
| `ModelRef.thinking_level` | `src/domain/value_objects.rs` | Optional alias default (`serde "thinking-level"`) |
| `ModelRegistryError::InvalidThinkingLevel { alias, value }` | `src/domain/value_objects.rs` | Registry parse error (degrades to warning + empty registry at load) |
| `AgentProfile.thinking_level` / `with_thinking_level` | `src/domain/knot_file.rs` | Profile override field + fluent builder |
| `AgentProfileError::InvalidThinkingLevel(String)` | `src/domain/knot_file.rs` | Profile parse error |
| `AgentConfig.thinking_level` | `src/domain/value_objects.rs` | Effective level carried to the CLI |
| `AgentProfile::resolve_for_knot` | `src/domain/value_objects.rs` | `profile.or(alias)` (model-ref) / profile-only (direct-spec) |
| `AgentConfig::build_cli_args` | `src/domain/value_objects.rs` | `--thinking <level>` emission (after `--model`, before `--tools`) |
| `RigStateProfile.thinking_level` | `src/domain/entities.rs` | State key (skip-if-none) |
| `write_state.rs` effective-level mapping | `src/application/usecases/write_state.rs` | State snapshot shows the effective level |
| models.yml template comment | `src/server.rs` | Auto-created template documents the key |

## Testing

| Test | Pins |
|---|---|
| `thinking_level_*` serde/Display/parse unit tests (value_objects) | Six tokens round-trip; `xhigh` rename; unknown tokens rejected |
| `ModelRegistry::from_yaml` thinking-level tests (value_objects) | Alias default parsed / absent → `None` / invalid → `InvalidThinkingLevel` naming alias + value |
| `parse_agent_profile` thinking-level tests (knot_file) | Frontmatter field parsed / default `None` / invalid → `InvalidThinkingLevel` |
| `build_cli_args` thinking-level tests (value_objects) | `--thinking <level>` after `--model`, before `--tools`; emitted for `off`; omitted when unset |
| `resolve_for_knot` hierarchy matrix (value_objects) | Four-case (profile × alias) matrix; direct-spec unconsulted (same-named alias with different level); alias-without-level lets the profile's through |
| `write_state` thinking-level tests | Effective level on state profile entries: profile-only, alias-default, profile-override, omitted-when-neither (key absent, not null) |
| `tests/thinking_level.rs` (acceptance) | Full flow through the **real adapters**: `model-ref` profile overriding its alias default, resolved from a real `models.yml`; `PiStdioAgentRunner` **and** `PiJsonAgentRunner` spawn an argv-recording mock CLI (`KNOT_TEST_CLI_PATH` pattern); both argvs carry `--thinking xhigh` after `--model` |

## Notes

- **Both runners are covered because both build from
  `build_cli_args()`** — the acceptance test asserts the flag in both
  spawned argvs even though neither adapter file changed; the pattern
  guards against a future runner bypassing the shared builder.
- **The mock CLI** is a two-line bash script: `printf '%s\n' "$@"`
  records argv (one arg per line); `cat` echoes stdin back as the
  agent's stdout so both runners see a successful run.
- **No capability validation, ever, by design** — pi clamps; adding
  capability checks would couple Knot to provider model metadata for a
  behaviour pi already handles.
- **`process_strand.rs` is unchanged** — it already resolves via
  `resolve_for_knot`; only the state snapshot needed the effective-level
  mapping.

## Related Documents

- [docs/release-notes.md](../../docs/release-notes.md) — v0.36.0 entry
- `knot-update` skill changelog — Knot 0.36.0 entry (no migration
  required; optional fields, adoption steps)
- `knot-create` skill — profile frontmatter table + models.yml section
  (override example)
- `knot-inspect` skill — profile listing (effective `thinking-level`,
  absent-key convention)
- [design-knot-step.md](design-knot-step.md) — the queue/stepping
  subsystem this plan does not touch
