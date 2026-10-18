# Plan 096: Hard-Coded Tie-Off Contract — the Content-Only Override

**Implementation Status:** ✅ Complete (2026-10-16) — released in v0.60.0

**Supersedes (in part):** [Plan 095 — Skill-Backed Tie-Off and
Event-Content Instructions](../plan-skill-backed-tieoff-instructions.md)
(the per-rig override surface: two full-file replacements → one
content-section replacement; the hard-coded default, the config-link
mechanics, the per-session pull, and the `knot-template` skill all remain).

**Depends on / sibling to:** [Plan 094 — Event-Enforcement Follow-Up
Tie-Off](../plan-bugfix-event-enforcement-followup-tieoff.md) — no change
needed there: the enforcement follow-up re-injects the run's
`listener_context`, which after this plan always carries the hard-coded
core.

## Problem

Plan 095 externalised the **whole** contract: both instruction texts
(`knot-tie-off.md` — the tie-off contract, event format, and rules — and
`knot-event-content.md` — the event-content guide) became full-file
per-rig replacements (`tie-off-instruction-file` /
`event-content-instruction-file`). What was wanted was externalising a
**part** of the skill: the user should be able to set the **content**
contract, not the contract itself.

Consequences of the over-externalisation:

- A rig override can delete the mechanics from the producer's prompt —
  the "final response *is* the tie-off" rule, the structured block
  format, and the "emit a block for every listed event, even when
  `occurred: false`" rule — the very contract that event enforcement
  tests the agent's *response* against.
- Plan 095's own documented risk ("a user override that changes the
  block format only breaks enforcement if the agent follows the
  override's format — that is the user's choice") is precisely the
  trade-off the user does not want.
- The fixed event properties (`event:`, `occurred:`, `timestamp:`) are
  carried only by wording a rig can replace.

## Target

The injected `# Subscriber Events` block is split into a **hard-coded
core** (the Knot agent's contract — same mechanics as today, never
overridable) and **one exposed content section** (what the user can
set):

| Segment | Source | Per-rig override? |
|---|---|---|
| Subscriber event list (the `- \`EventId\` — …` bullets) | code (dynamic) | — (unchanged) |
| `## Tie-Off Contract` — the final response *is* the tie-off; never read/write `tie-offs/` directly | hard-coded in the binary | **No** |
| `## Event Format` — the fixed block properties `event:`, `occurred:`, `timestamp:` (+ the optional subscriber-declared fields marker) | hard-coded in the binary | **No** |
| `## Rules` — one block per listed event, **always** (mentioning in narrative is not an acknowledgement; `occurred: false` still counts) | hard-coded in the binary | **No** |
| **The tie-off content section** — `## Tie-Off Content` (the tie-off's header/narrative content) + `## Event Content Guide` (how to fill in `description`, the body content, and additional custom properties) | hard-coded default; a per-rig file link replaces **this section only** | **Yes — the only exposed part** |

Properties that must survive any override, and where they live after
this plan:

- **Events are always emitted** — `## Rules` is hard-coded; a rig can no
  longer remove it.
- **`event: EventId` / `occurred: true` / `timestamp: …`** — the `##
  Event Format` worked example is hard-coded with exactly these
  properties.
- **`description`, the body content, and additional custom
  properties** — instructed by the (overridable) content section; the
  user's file is where per-project content expectations live.

The **mechanics** stay exactly the same as Plan 095: the default is
compiled into the binary (`include_str!` of the skill's template files);
the override is a file link in `rig/.workspace-agent-config.yaml`
(absolute / rig-relative / `~/...` / `file://`; `http(s)` refused);
pulled fresh on every new session, never cached; a missing/unreadable
link falls back to the hard-coded default with a
`[KNOT][EVENT] type=CONFIG` service-log line; the service never writes
into the rig. Only the *surface* changes: two keys → one, and the
linked file reaches only the content section.

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| The exposed surface | **One** content section, **one** config key `tie-off-content-instruction-file`, **one** template `knot-tie-off-content.md` | "only a tie-off content section should be exposed": the tie-off's header content and the per-event content details (`description`, body, additional properties) are both *content* — one overridable unit |
| The hard-coded core | The existing `knot-tie-off.md` (contract + format + rules) stays the single source of truth, still `include_str!`-ed — now **never overridable** | No new split required: the file already carries the three hard-coded sections verbatim; its content is unchanged by this plan |
| The old `tie-off-instruction-file` key | **Retired** — kept parse-only so its presence can be detected, then warned about; no longer honoured | Its purpose (overriding the contract) no longer exists; silently ignoring it would lose a rig's configuration without notice |
| The old `event-content-instruction-file` key | **Retired** the same way; migration = relink the file under `tie-off-content-instruction-file` | The section it now covers is broader than "event content" (it also carries the tie-off header guidance), so the old name would mislead |
| Default content section | New template `knot-tie-off-content.md` = `## Tie-Off Content` (new default guidance for the tie-off header/narrative) + `## Event Content Guide` (the existing guide text, verbatim) | The user's header-content customisation needs a default section to customise; the event-content guidance moves into it unchanged |
| `knot-tie-off.md` in the skill | Kept, **reference-only** — the shipped contract, readable by humans; the skill no longer copies it into the project | The binary still compiles it in (single source of truth); the rig owns no copy of the contract |
| Enforcement | Parser unchanged | The agent's response is still parsed against the structured block format — but the agent now *always* sees the canonical format and rules, so the override/format divergence risk from Plan 095 is eliminated by construction |
| Prompt shape | Unchanged order: core block, then content section, then `---`; subscriber bullets and the `# Subscriber Events` header byte-identical | Protects `extract_emitted_event_ids` (anchors on the subscriber bullet run) and any prompt-shape assertions |
| Version | 0.59.0 → **0.60.0** | Breaking change to the config surface (two keys retired, one added) — a Knot release with a `knot-update` migration entry |

## The new default content template

`knot-tie-off-content.md` (replaces `knot-event-content.md`; created in
Phase 0) — the default for the overridable section:

```markdown
## Tie-Off Content

Your tie-off (final response) begins with the content of the work done in this
session — a short, factual summary: what was done or found, the key decisions
and their rationale, and pointers to the files changed or produced. Keep it
thin — facts and pointers, not a re-telling of the session.

## Event Content Guide
How to fill in each event block:

- `description` — explain **why** the event was (or wasn't) triggered, plus any additional
  requested information. One or two focused lines: the facts a consumer needs to act,
  not a re-statement of the session.
- `timestamp` — required when `occurred: true` (ISO 8601, the moment the event occurred).
- **Additional fields** — when the event description above specifies extra fields (e.g.
  `next-task-context`, `plan-id`), include them in the frontmatter when `occurred: true`,
  in the shape the description asks for.
- **Body** — the text after the closing `---` is freeform narrative context about the event:
  pointers to files, decisions and their rationale, and anything the consumer needs beyond
  the frontmatter. Keep it thin — facts and pointers, not narrative.
- A block with `occurred: false` still needs a `description` saying why it did not occur;
  its body is optional.
```

(`knot-tie-off.md` — the hard-coded core — is **unchanged**.)

## Phases

### Phase 0 — Domain: the hard-coded core + one content parameter

**Files:** `.agents/skills/knot-template/knot-tie-off-content.md` (new),
`.agents/skills/knot-template/knot-event-content.md` (deleted),
`src/domain/events.rs`

- [x] Create `knot-tie-off-content.md` with the default content template
      above (the old `knot-event-content.md` stays for now — it is still
      `include_str!`-ed)
- [x] `src/domain/events.rs` — `DEFAULT_EVENT_CONTENT_INSTRUCTION` →
      `DEFAULT_TIE_OFF_CONTENT_INSTRUCTION` = `include_str!` of the new
      template; `DEFAULT_TIE_OFF_INSTRUCTION` unchanged (contract +
      format + rules) with its doc comment updated: **never
      overridable** — the per-rig link reaches only the content section;
      **then** delete `knot-event-content.md` (no longer referenced —
      the three items land in one commit so the build is green throughout)
- [x] `PromptInstructionSource` — one method:
      `read_tie_off_content_instruction(&self) -> Option<String>`
      (`None` → the embedded default content section)
- [x] `build_listener_context_with_instructions` — drop the
      `tie_off_instruction` parameter: the core block
      (`DEFAULT_TIE_OFF_INSTRUCTION`) is **always** injected; the
      content section = the parameter or
      `DEFAULT_TIE_OFF_CONTENT_INSTRUCTION`; `BuildContext` doc
      comments updated to match
- [x] Domain tests: a `Some` content override replaces **only** the
      content section — assert the hard-coded markers
      (`## Tie-Off Contract`, `## Event Format`, `## Rules`, the
      `event: EventId` / `occurred: true` example, and the
      "Emit exactly one event block per subscriber event" rule) are
      present **and** the default content-section text is absent when a
      custom section is injected; `None` → the full default (snapshot
      regression, current output modulo the new `## Tie-Off Content`
      default section); the subscriber-bullet byte-identity guard
      (protects `extract_emitted_event_ids`) stays green

### Phase 1 — Config + adapter: the one key, the two retired keys

**Files:** `src/domain/value_objects.rs`,
`src/adapters/outbound/prompt_instruction_source.rs`

- [x] `RigAgentConfig` — new field `tie_off_content_instruction_file`
      (rename `tie-off-content-instruction-file`, `default_none_string`,
      doc comment: full replacement of the default **content section
      only**; the contract/format/rules are hard-coded); the two old
      fields stay **parse-only** (renamed `deprecated_*`, same
      serde renames, same default) so their presence is detectable —
      doc comments: retired in 0.60.0
- [x] Adapter — one public read
      (`read_tie_off_content_instruction`): fresh config parse per call;
      a present `tie-off-instruction-file` warns once per call —
      "deprecated in Knot 0.60.0 and no longer honoured — the tie-off
      contract, event format, and rules are hard-coded; remove the
      key"; a present `event-content-instruction-file` warns —
      "deprecated in Knot 0.60.0 — relink the file under
      tie-off-content-instruction-file (the tie-off content section)";
      both as `[KNOT][EVENT] type=CONFIG source=prompt-instructions`
      lines; neither value is read as content
- [x] Keep: relative / absolute / `~/` / `file://` resolution;
      `http(s)` refusal with a warning; missing/unreadable link →
      `None` + warning; fresh re-read per call; malformed config →
      `None` + warning
- [x] Adapter tests: the new key honoured in all four link forms;
      deprecated keys present → **not** used for the content section;
      each deprecated key still emits its warning (observable via
      the parse, not the content); no leak across keys; re-pull per
      call (config and linked file); malformed config → `None`;
      legacy config without the new key parses unchanged

### Phase 2 — Wiring

**Files:** `src/application/usecases/context_providers.rs`,
`src/server.rs`

- [x] `AgentEventsContextProvider::build_full_context` — one fetch
      (`read_tie_off_content_instruction`) passed to
      `build_listener_context_with_instructions` (one instruction
      parameter now)
- [x] `server.rs` — the auto-created
      `.workspace-agent-config.yaml` header comment: document
      `tie-off-content-instruction-file` (commented-out example:
      `knot-template/knot-tie-off-content.md`), note that the two old
      keys are retired (existing files keep parsing; the adapter
      warns); existing rigs' files are untouched (the template is
      written only when the file is missing — unchanged behaviour)
- [x] Application tests: the counting mock source proves one fetch per
      session (two consecutive `process_strand` executions → two
      reads); a file edit between executions changes the second
      prompt's **content section only** (core block byte-identical)

### Phase 3 — The `knot-template` skill + `AGENTS.md`

**Files:** `.agents/skills/knot-template/SKILL.md`,
`.agents/skills/knot/SKILL.md`, `AGENTS.md`

- [x] Rewrite `knot-template/SKILL.md` (version 1.0.0 → 2.0.0):
      the mechanism — hard-coded core (contract / event format /
      rules, **not overridable per rig**) + the one exposed content
      section + the one key + per-session pull + fallback + "the
      service writes nothing into the rig"; the assets table —
      `knot-tie-off.md` = the shipped contract (**reference only**,
      never copied into the project) and `knot-tie-off-content.md` =
      the default content section (the only override starting point);
      the procedure — copy **only** the content template into
      `rig/knot-template/` and merge the **one** key
      (`tie-off-content-instruction-file`) into the config; a
      **migration note** for 0.59.0 rigs (see Migration below)
- [x] `knot` master — the `knot-template` routing row's wording
      ("tie-off / event-content templates" → the contract (hard-coded)
      + the content section (overridable))
- [x] `AGENTS.md` — the `knot-template` skill-list bullet (the two
      companion template files → the contract reference + the content
      template); the companion-files diff loop:
      `knot-tie-off.md knot-event-content.md` →
      `knot-tie-off.md knot-tie-off-content.md`

### Phase 4 — Docs, release notes, `knot-update`

**Files:** `docs/concepts.md`, `docs/configuration/rig-structure.md`,
`docs/release-notes.md`, `.agents/skills/knot-update/SKILL.md`,
`project/plans/master-plan.md`

- [x] `docs/concepts.md` — rewrite the skill-backed paragraph: the
      hard-coded core (contract / event format / rules — not
      overridable), the one exposed content section and its key, the
      per-session pull, the two retired keys
- [x] `docs/configuration/rig-structure.md` — the config tree line and
      the overrides section: one key; the old keys retired (present →
      parsed, warned, not honoured)
- [x] `docs/release-notes.md` — the v0.60.0 entry (breaking: the two
      keys retired with warnings; added: the exposed tie-off content
      section with default header guidance, the single
      `tie-off-content-instruction-file` key)
- [x] `knot-update` (skill version 1.25.0 → 1.26.0) — the 0.60.0
      changelog entry: what changed in project documents and the
      migration (below)
- [x] `master-plan.md` — the 096 row + overview (this plan)

### Phase 5 — Verification + release

- [x] `cargo test` (full suite)
- [x] `cargo clippy` — no new warnings
- [x] Version bump 0.59.0 → 0.60.0 (`Cargo.toml` + release notes);
      `cargo install --path . --locked`; `knot --version` reports
      0.60.0 (per AGENTS.md — after `project-plan-completion` bumps
      the version)
- [x] Deploy the updated `knot-template` (whole directory — now the
      contract reference + the content template), the updated `knot`
      master, and `knot-update` to `~/.agents`, diff-verify (including
      the new companion file per the updated `AGENTS.md` loop)

Per AGENTS.md: **no live rig runs in this repo** — verification via
`cargo test` / `cargo clippy` and the mock-CLI harness only.

## Verification

- The full `cargo test` suite — the domain snapshot (core always
  present, override reaches only the content section), the adapter
  (one key, two retired keys warned), and the application counting
  tests (one fetch per session).
- `cargo clippy` — clean.
- Prompt-shape checks at the unit level (no live rig runs): with no
  config, the `# Subscriber Events` block contains the core
  (`## Tie-Off Contract` / `## Event Format` / `## Rules`) + the
  default content section (`## Tie-Off Content` / `## Event Content
  Guide`) in that order; with a content link set, the core is
  **byte-identical** and only the content section differs.

## Migration / deprecation (for 0.59.0 rigs)

| 0.59.0 artefact | 0.60.0 behaviour |
|---|---|
| `tie-off-instruction-file` in `rig/.workspace-agent-config.yaml` | Parsed but **not honoured**; a `[KNOT][EVENT] type=CONFIG` warning per new session. **Remove the key.** Any customisation in the linked file that the rig needs as *content* must move into a `tie-off-content-instruction-file` file (the core sections in it are ignored by design — the hard-coded core is always what the agent sees) |
| `event-content-instruction-file` in the config | Parsed but **not honoured**; warning per new session. **Re-link the same file under `tie-off-content-instruction-file`** (optionally rename it to `knot-tie-off-content.md`) |
| `rig/knot-template/knot-tie-off.md` (the copied contract) | Inert — never read. Delete it (or keep it as local documentation) |
| `rig/knot-template/knot-event-content.md` (custom content) | Inert until re-linked under the new key (see above) |

The migration is user/agent-driven (the `knot-update` entry + the
`knot-template` skill's migration note); the binary changes nothing in
the rig.

## Out of scope

- Per-knot override keys (there is no per-knot config file; unchanged).
- The enforcement parser (response-side parsing of the structured
  blocks — unchanged; the prompt-side guarantee is what this plan
  adds).
- `HANDOFF_NOTE` / `TASKS_INCOMPLETE_DESCRIPTION` / retry-note wording
  (still hard-coded literals — separate concern, as in Plan 095).
- No-subscriber baseline injection (still: no injection at all when no
  listeners — a separate plan).
- The tie-off **file** shape the service writes (the `## <knot>
  triggered by …` header, `Timestamp:`, `session:`) — unchanged; the
  content section governs only the agent's *response* content.
