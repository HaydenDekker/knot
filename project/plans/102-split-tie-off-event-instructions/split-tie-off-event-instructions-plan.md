# Plan 102: Split Tie-Off and Event-Content Instructions — the Two Instruction Documents with Fixed Injection Scope

**Implementation Status:** ✅ Complete (2026-11-01) — released in v0.65.0. Phases 0–5 delivered: the two hard-coded cores + two embedded defaults + `build_tie_off_context` (0), the two config keys + per-section fallback (1), the unconditional `# Tie-Off` block + consumer-gated `# Subscriber Events` block (2), the `knot-template` v3 / master routing / docs (3–4), version bump to 0.65.0 (5). Full suite green (1208 passed, 0 failed), clippy at baseline.

**Revises (in part):** [Plan 096 — Hard-Coded
Tie-Off Contract](../096-hardcoded-tieoff-contract/hardcoded-tieoff-contract-plan.md)
(the per-rig override surface: one content section → **two**
instruction documents; the hard-coded-core split, the config-link
mechanics, the per-session pull, the fallback behaviour, and the
`knot-template` skill all remain). Plan 096's own "Out of scope" line —
"no-subscriber baseline injection (still: no injection at all when no
listeners — a separate plan)" — is this plan.

## Problem

Plan 096 externalised a **single** overridable content section
(`tie-off-content-instruction-file` → `knot-tie-off-content.md`), which
carries **two** distinct concerns under one roof:

- `## Tie-Off Content` — how to write the **tie-off** (the agent's
  final response).
- `## Event Content Guide` — how to fill in the **event blocks**
  (`description`, the body, additional custom properties).

And the whole `# Subscriber Events` block — hard-coded core
(`## Tie-Off Contract` / `## Event Format` / `## Rules`) **and** the
content section — is only built when the knot **has event consumers**
(or a self-continuation watermark entry):

```rust
// No listeners and no self-continuation — no injection needed.
if matching_knots.is_empty() && self_continuation_desc.is_none() {
    return String::new();
}
```

So a **leaf knot — one that emits no events** (e.g. an orchestrator
that re-kicks the pipeline by writing event files and stops the
service) receives **none** of the tie-off instruction: not the
overridable content section, not even the hard-coded
"your final response *is* your tie-off — never read/write `tie-offs/`
files" contract. The tie-off concern was wrongly scoped to producer
status.

**Field evidence:** `budget-app-core` (software-factory rig, Knot
0.64.0), 2026-11-01 21:26–21:30 — the final `QueueIdle` run of
`orchestrator-loom/pipeline-on-queue-idle` (session
`01a196a5-9cb2-7456-b1b3-a2e89714d75e`). The rig's
`tie-off-content-instruction-file` link was healthy (file present,
readable, byte-identical in the shim and external rig; no
`type=CONFIG` fallback line in the service log) — yet the session
transcript's opening prompt contains **zero** occurrences of
`Subscriber Events`, `Tie-Off Contract`, `Tie-Off Content`, or
`Event Content Guide`: the knot declares no `event:` subscribers and no
knot subscribes to it, so `build_listener_context_with_instructions`
returned the empty string. The rig's customised pointer-not-copy
tie-off discipline never reached the agent; the tie-off came back as a
full narrative copy instead of one-line-status-plus-paths. Every
consumer-bearing knot in the same rig received the block correctly —
the gap is exactly the no-consumer path.

## Target

Two overridable **documents**, each with a **fixed injection scope**:

| Document | Section | Config key | Default template | Injection scope |
|---|---|---|---|---|
| **Tie-off instruction** | `## Tie-Off Content` | `tie-off-instruction-file` (**resurrected** — the 0.59.0 name) | `knot-tie-off-content.md` (now this section only) | **Every knot, every new session — unconditional** |
| **Event-content instruction** | `## Event Content Guide` | `event-content-instruction-file` (**resurrected** — the 0.59.0 name) | `knot-event-content.md` (**resurrected** name; now this section only) | **Only when the knot has event consumers** (or a self-continuation entry) — i.e. only when the `# Subscriber Events` block is built |

The **hard-coded core splits along the same scope line**. Today's
`knot-tie-off.md` (contract + format + rules, one constant) becomes two
constants, both `include_str!`-ed, both never overridable:

| Core segment (today's `knot-tie-off.md`) | New source file | Scope |
|---|---|---|
| The universal sentence — "Your final response **is** your tie-off: … never write or read directly from `tie-offs/` files — deliver your output as your final response." (second paragraph of `## Tie-Off Contract`, minus the event-block clause) | `knot-tie-off-contract.md` (new) | **Unconditional** — the `# Tie-Off` block |
| The subscriber-acknowledgement paragraph ("You have a number of subscribers … delivered to them via your tie-off.") + `## Event Format` + `## Rules` | `knot-event-contract.md` (new) | **Consumer-gated** — the `# Subscriber Events` block |

**Prompt shape (new)** — both blocks are prepended to the knot's
instructions, as today; `# Tie-Off` first:

```
# Tie-Off

## Tie-Off Contract
<universal sentence — hard-coded>

## Tie-Off Content
<doc 1 — per-session pull; embedded default when no link / unreadable>
---

# Subscriber Events              ← only when the knot has consumers (or a self-continuation entry)
The following event/s have been declared by subscribers:
- `EventId` — description
…
## Event Acknowledgement
<acknowledgement paragraph — hard-coded>
## Event Format
<hard-coded, unchanged>
## Rules
<hard-coded, unchanged>
## Event Content Guide
<doc 2 — per-session pull; embedded default when no link / unreadable>
---

<knot instructions…>
```

Everything else stays as it is: the per-session fresh pull (now **two**
pulls), link resolution (absolute / rig-relative / `~/...` /
`file://`; `http(s)://` refused), the fallback (a
missing/unreadable link never blanks its section — the embedded
default for **that section** is injected with a `[KNOT][EVENT]
type=CONFIG` service-log line; the other section is unaffected), the
service writing nothing into the rig, and the enforcement parser
(response-side) unchanged.

**Config surface** — in `rig/.workspace-agent-config.yaml`:

```yaml
# Optional full replacement for the default tie-off instruction
# (the tie-off header/narrative content). Injected into EVERY knot,
# every new session.
tie-off-instruction-file: knot-template/knot-tie-off-content.md

# Optional full replacement for the default event-content guide
# (how to fill in event blocks). Injected only when the knot has
# subscriber events.
event-content-instruction-file: knot-template/knot-event-content.md

# (0.60.0–0.64.x key — retired in 0.65.0: parsed, warned per session,
#  never honoured. Migration: split the linked file at the
#  `## Event Content Guide` heading; re-link the halves.)
```

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| The exposed surface | **Two** documents, **two** config keys, **two** templates — the tie-off instruction and the event-content instruction | The two sections of Plan 096's content template concern different artefacts with different scopes: the tie-off is every knot's final response; the event blocks exist only for knots that emit to subscribers. One overridable unit for two scopes is the spec miss this plan rectifies |
| Injection scope | The `# Tie-Off` block (core sentence + doc 1) is **unconditional**; the `# Subscriber Events` block (ack + format + rules + doc 2) keeps today's gate — no consumers and no self-continuation → not built | Fixes the leaf-knot gap (the orchestrator and any other terminal knot now get the tie-off contract + the rig's tie-off discipline); producer-knot behaviour is otherwise unchanged |
| Key names | The 0.59.0 names **resurrected** with exactly these meanings: `tie-off-instruction-file` (doc 1) and `event-content-instruction-file` (doc 2) | The names match the two documents; the 0.59.0→0.60.0 retirement was the step that folded the tie-off concern into the consumer-gated block |
| The 0.60.0–0.64.x combined key | `tie-off-content-instruction-file` **retired**: kept parse-only so its presence is detected and warned about per session; never honoured | Silently ignoring it would lose a rig's configuration without notice — the same posture Plan 096 applied to the 0.59.0 keys |
| The hard-coded core | Split into `DEFAULT_TIE_OFF_CONTRACT` (universal sentence) + `DEFAULT_EVENT_CONTRACT` (acknowledgement + `## Event Format` + `## Rules`); both `include_str!`-ed from new reference files; **never overridable** | The core is split on the same scope line as the documents; the "final response is the tie-off" rule must reach leaf knots, and the format/rules only make sense for knots that emit |
| `knot-tie-off.md` in the skill | Replaced by the two reference files (`knot-tie-off-contract.md`, `knot-event-contract.md`) — **reference only**, never copied into the project | The binary compiles both in (single source of truth); the rig owns no copy of the contract |
| Prompt order | `# Tie-Off` first, then `# Subscriber Events`; the marker line `The following event/s have been declared by subscribers:` and the contiguous `- \`EventId\`` bullet run are **preserved** | The agent sees the universal contract first; `extract_emitted_event_ids` (which anchors on that marker line and bullet run) is unaffected |
| Fallback | Per-section: a missing/unreadable `tie-off-instruction-file` falls back to the embedded default tie-off content **only**; the event-content guide is unaffected (and vice versa) | One section's link failure must not blank or alter the other section |
| Version | 0.64.0 → **0.65.0** | Breaking change to the config surface (one key retired, two keys resurrected/honoured) — a Knot release with a `knot-update` migration entry |

## The new default template texts

`knot-tie-off-contract.md` (new; the universal core, unconditional):

```markdown
## Tie-Off Contract
Your final response **is** your tie-off: tie-offs are managed by the
infrastructure service, so never write or read directly from `tie-offs/`
files — deliver your output as your final response.
```

`knot-event-contract.md` (new; the consumer-gated core — the
acknowledgement paragraph plus the existing `## Event Format` and
`## Rules` sections, verbatim from today's `knot-tie-off.md`):

```markdown
## Event Acknowledgement
You have a number of subscribers that have requested to be notified if
certain events occur during this session. You must acknowledge each
event in your tie-off (final response). Subscribers can't begin their
work until your events are delivered to them via your tie-off.

## Event Format
<verbatim from today's knot-tie-off.md, unchanged>

## Rules
<verbatim from today's knot-tie-off.md, unchanged>
```

`knot-tie-off-content.md` (edited; doc 1 — the `## Tie-Off Content`
section only, text unchanged from today):

```markdown
## Tie-Off Content

Your tie-off (final response) begins with the content of the work done in this
session — a short, factual summary: what was done or found, the key decisions
and their rationale, and pointers to the files changed or produced. Keep it
thin — facts and pointers, not a re-telling of the session.
```

`knot-event-content.md` (new — the resurrected 0.59.0 name; doc 2 — the
`## Event Content Guide` section only, text unchanged from today):

```markdown
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
- **Deduplication** — if a queued event (see the *Queued Events* section of your
  prompt, when present) already carries the same facts — compare the frontmatter
  keys such as plan/phase — mark the subscriber event `occurred: false` and say so
  in its `description` (it is already queued); do not queue a second signal for the
  same outcome.
```

`knot-tie-off.md` is **deleted** (its text is fully redistributed to
the two core files above; all template edits and the constant rewiring
land in one commit so the build is green throughout).

## Phases

### Phase 0 — Domain: the core split + two section parameters

**Files:** `.agents/skills/knot-template/knot-tie-off.md` (deleted),
`.agents/skills/knot-template/knot-tie-off-contract.md` (new),
`.agents/skills/knot-template/knot-event-contract.md` (new),
`.agents/skills/knot-template/knot-tie-off-content.md` (edited —
`## Event Content Guide` removed),
`.agents/skills/knot-template/knot-event-content.md` (new),
`src/domain/events.rs`

- [ ] Create the two core reference files (`knot-tie-off-contract.md`,
      `knot-event-contract.md`) with the texts above
- [ ] Split `knot-tie-off-content.md` to the `## Tie-Off Content`
      section only; create `knot-event-content.md` with the
      `## Event Content Guide` section (text verbatim from today's
      combined template); **then** delete `knot-tie-off.md` (no longer
      `include_str!`-ed)
- [ ] `src/domain/events.rs` — constants: `DEFAULT_TIE_OFF_INSTRUCTION`
      → `DEFAULT_TIE_OFF_CONTRACT` (the universal file) +
      `DEFAULT_EVENT_CONTRACT` (the consumer-gated file);
      `DEFAULT_TIE_OFF_CONTENT_INSTRUCTION` → `include_str!` of the
      split `knot-tie-off-content.md`; `DEFAULT_EVENT_CONTENT_INSTRUCTION`
      → `include_str!` of `knot-event-content.md` (name resurrected);
      doc comments updated: the core is **never overridable** per rig;
      the per-rig links reach the two content sections only
- [ ] `PromptInstructionSource` — one method
      (`read_tie_off_content_instruction`) → **two methods**:
      `read_tie_off_instruction(&self) -> Option<String>` and
      `read_event_content_instruction(&self) -> Option<String>`
      (`None` → the embedded default for that section)
- [ ] Builders — `build_listener_context_with_instructions` → two
      builders: `build_tie_off_context(tie_off_content_instruction:
      Option<&str>) -> String` (**never empty**: `# Tie-Off` header,
      `## Tie-Off Contract` + `DEFAULT_TIE_OFF_CONTRACT`, the content
      section (parameter or `DEFAULT_TIE_OFF_CONTENT_INSTRUCTION`),
      closing `---`) and
      `build_listener_context_with_instructions(knot, loom_id,
      all_knots, self_continuation_desc, event_content_instruction:
      Option<&str>) -> String` (the **same gate as today** — no
      consumers and no self-continuation → empty string; otherwise the
      `# Subscriber Events` block: marker line + bullets **unchanged**,
      then `DEFAULT_EVENT_CONTRACT` (acknowledgement / format / rules),
      then the event-content section (parameter or
      `DEFAULT_EVENT_CONTENT_INSTRUCTION`), closing `---`); the
      `## Tie-Off Contract` heading appears exactly once per prompt;
      `BuildContext` doc comments updated to match
- [ ] Domain tests: **leaf knot** (no consumers, no self-continuation)
      — `build_listener_context_with_instructions` → empty string,
      `build_tie_off_context` → the full `# Tie-Off` block with both
      default sections; **consumer knot** — both blocks present,
      `# Tie-Off` before `# Subscriber Events`; the marker line and
      the contiguous bullet run byte-identical (protects
      `extract_emitted_event_ids`); a `Some` doc-1 override replaces
      **only** the tie-off content (assert `## Event Content Guide`
      default text still present, `## Tie-Off Contract` + the event
      format/rules markers present); a `Some` doc-2 override replaces
      **only** the event content guide; both `None` → full defaults
      (snapshot regression, current output modulo the split); the
      self-continuation (`TasksIncomplete`) entry still yields the
      event block with its implicit consumer

### Phase 1 — Config + adapter: the two resurrected keys, the one retired key

**Files:** `src/domain/value_objects.rs`,
`src/adapters/outbound/prompt_instruction_source.rs`

- [ ] `RigAgentConfig` — the two previously parse-only fields become
      **honoured**: `tie_off_instruction_file` (rename
      `tie-off-instruction-file`, doc comment: full replacement of the
      default **tie-off instruction section only**; injected into
      every knot; the contract sentence is hard-coded) and
      `event_content_instruction_file` (rename
      `event-content-instruction-file`, doc comment: full replacement
      of the default **event-content guide only**; injected only when
      the knot has subscriber events); the previously honoured field
      becomes **parse-only**: `tie_off_content_instruction_file`
      (same serde rename, same default) — doc comment: retired in
      0.65.0, the combined section is split into the two keys
- [ ] Adapter — two public reads (`read_tie_off_instruction` /
      `read_event_content_instruction`): fresh config parse per call;
      a present `tie-off-content-instruction-file` warns per call —
      "retired in Knot 0.65.0 and no longer honoured — split the
      linked file at the `## Event Content Guide` heading: the top
      half re-links under `tie-off-instruction-file`, the bottom half
      under `event-content-instruction-file`" — as a
      `[KNOT][EVENT] type=CONFIG source=prompt-instructions` line; the
      value is never read as content; the 0.60.0 deprecation warnings
      for the two resurrected keys are **removed**
- [ ] Per-key: keep relative / absolute / `~/` / `file://` resolution;
      `http(s)` refusal with a warning; missing/unreadable link →
      `None` + warning; fresh re-read per call; malformed config →
      `None` + warning — **independently per key** (a failure of one
      link never affects the other read)
- [ ] Adapter tests: each new key honoured in all four link forms;
      each section falls back to its own embedded default on its own
      link failure; no leak across keys; the retired key present →
      **not** used for either section, and emits its warning
      (observable via the parse, not the content); re-pull per call
      (config and linked files); malformed config → `None`; a
      0.60.0-era config (only the combined key) parses unchanged

### Phase 2 — Wiring

**Files:** `src/application/usecases/context_providers.rs`,
`src/server.rs`

- [ ] `AgentEventsContextProvider::build_full_context` — **two** fetches
      (`read_tie_off_instruction`, `read_event_content_instruction`);
      the `# Tie-Off` block is **always** built; the `# Subscriber
      Events` block only when non-empty (today's gate); the pending
      events scan stays inside the consumer-gated path (it anchors on
      the marker line, which is unchanged) and runs only when the
      listener block was built; concatenation order: tie-off block,
      listener block, pending section
- [ ] `server.rs` — the auto-created `.workspace-agent-config.yaml`
      header comment: document the **two** keys (commented-out
      examples: `knot-template/knot-tie-off-content.md` /
      `knot-template/knot-event-content.md`), note that
      `tie-off-content-instruction-file` is retired (existing files
      keep parsing; the adapter warns); existing rigs' files are
      untouched (the template is written only when the file is
      missing — unchanged behaviour)
- [ ] Application tests: the counting mock source proves **two**
      fetches per session (two consecutive `process_strand`
      executions → four reads); a **leaf knot's** prompt contains the
      `# Tie-Off` block and **no** `# Subscriber Events` block; a
      **consumer knot's** prompt contains both, in order, plus the
      pending section where applicable; a file edit between
      executions changes the second prompt's **affected section
      only** (the other section and both hard-coded cores
      byte-identical); the event-enforcement follow-up path (Plan
      094's re-injection of the run's listener context) picks up the
      new two-block shape

### Phase 3 — The `knot-template` skill, master router, `AGENTS.md`

**Files:** `.agents/skills/knot-template/SKILL.md`,
`.agents/skills/knot/SKILL.md`, `AGENTS.md`

- [ ] Rewrite `knot-template/SKILL.md` (version 2.1.0 → 3.0.0):
      the mechanism — **two** documents with **fixed** injection
      scopes (the tie-off instruction: every knot, unconditional; the
      event-content guide: only when the knot has subscriber events)
      + the hard-coded core split on the same line (the universal
      contract sentence / the acknowledgement + format + rules,
      neither overridable) + the **two** keys + the per-session pull
      (two pulls) + the **per-section** fallback + "the service
      writes nothing into the rig"; the assets table —
      `knot-tie-off-contract.md` + `knot-event-contract.md` = the
      shipped core (**reference only**, never copied into the
      project) and `knot-tie-off-content.md` + `knot-event-content.md`
      = the two default documents (the override starting points);
      the procedure — copy **both** templates into
      `rig/knot-template/` and merge the **two** keys into the
      config; a **migration note** for 0.60.0–0.64.x rigs (see
      Migration below)
- [ ] `knot` master — the `knot-template` routing row's wording
      (the contract (hard-coded) + the content section (overridable)
      → the two hard-coded core references + the two overridable
      instruction documents)
- [ ] `AGENTS.md` — the `knot-template` skill-list bullet (the two
      companion template files → the two core references + the two
      content templates); the companion-files diff loop:
      `knot-tie-off.md knot-tie-off-content.md` →
      `knot-tie-off-contract.md knot-event-contract.md
      knot-tie-off-content.md knot-event-content.md`

### Phase 4 — Docs, release notes, `knot-update`, master plan

**Files:** `docs/concepts.md`, `docs/configuration/rig-structure.md`,
`docs/release-notes.md`, `.agents/skills/knot-update/SKILL.md`,
`project/plans/master-plan.md`

- [ ] `docs/concepts.md` — rewrite the skill-backed paragraph: the
      two documents and their fixed injection scopes, the hard-coded
      core split (universal sentence / acknowledgement + format +
      rules — not overridable), the two keys, the per-session pull,
      the retired combined key
- [ ] `docs/configuration/rig-structure.md` — the config tree lines
      and the overrides section: two keys; the combined key retired
      (present → parsed, warned, not honoured)
- [ ] `docs/release-notes.md` — the v0.65.0 entry (breaking:
      `tie-off-content-instruction-file` retired with a warning;
      added: the `# Tie-Off` block injected into **every** knot, the
      two resurrected keys)
- [ ] `knot-update` — the 0.65.0 changelog entry: what changed in
      project documents and the migration (below); skill version bump
- [ ] `master-plan.md` — the 102 row + overview (this plan) + the
      Last-Updated header line

### Phase 5 — Verification + release

- [ ] `cargo test` (full suite)
- [ ] `cargo clippy` — no new warnings
- [ ] Version bump 0.64.0 → 0.65.0 (`Cargo.toml` + release notes);
      `cargo install --path . --locked`; `knot --version` reports
      0.65.0 (per AGENTS.md — after `project-plan-completion` bumps
      the version)
- [ ] Deploy the updated `knot-template` (whole directory — now the
      two core references + the two content templates), the updated
      `knot` master, and `knot-update` to `~/.agents`, diff-verify
      (including all four companion files per the updated
      `AGENTS.md` loop)

Per AGENTS.md: **no live rig runs in this repo** — verification via
`cargo test` / `cargo clippy` and the mock-CLI harness only.

## Verification

- The full `cargo test` suite — the domain split (leaf knot: tie-off
  block present, listener block empty; consumer knot: both blocks,
  order, byte-identity of the marker/bullets and the cores; per-
  section overrides), the adapter (two honoured keys, one retired key
  warned, independent fallbacks), and the application counting tests
  (two fetches per session).
- `cargo clippy` — clean.
- Prompt-shape checks at the unit level (no live rig runs): with no
  config, a leaf knot's prompt carries `# Tie-Off` (`## Tie-Off
  Contract` / `## Tie-Off Content`) only; a consumer knot carries
  `# Tie-Off` then `# Subscriber Events` (marker + bullets, `## Event
  Acknowledgement` / `## Event Format` / `## Rules`, `## Event Content
  Guide`); with a link set, only the linked section differs — every
  core is byte-identical.

## Migration / deprecation (for 0.60.0–0.64.x rigs)

| 0.60.0+ artefact | 0.65.0 behaviour |
|---|---|
| `tie-off-content-instruction-file` in `rig/.workspace-agent-config.yaml` | Parsed but **not honoured**; a `[KNOT][EVENT] type=CONFIG` warning per new session. **Split the linked file at the `## Event Content Guide` heading**: the top half → `knot-template/knot-tie-off-content.md`, re-linked under `tie-off-instruction-file`; the bottom half → `knot-template/knot-event-content.md`, re-linked under `event-content-instruction-file`. Remove the old key. |
| `rig/knot-template/knot-tie-off-content.md` (the 0.60.0-era combined file) | Inert until split and re-linked (see above). |
| Legacy 0.59.0 rigs: `tie-off-instruction-file` linked to a **full-contract** file | The key is honoured again (the 0.60.0 warning is removed) — but the file's content now replaces only the tie-off **content** section. Point it at a content-only file (or the default template) so the agent isn't shown the old full contract as content. |

The migration is user/agent-driven (the `knot-update` entry + the
`knot-template` skill's migration note); the binary changes nothing
in the rig. **Worked example (the field-evidence rig):**
`budget-app-core` — split `software-factory-rig/knot-template/knot-tie-off-content.md`
at the `## Event Content Guide` heading (top half stays
`knot-tie-off-content.md`; bottom half becomes `knot-event-content.md`),
replace the combined key with the two keys in the rig's
`.workspace-agent-config.yaml` (the shim's local config **and** the
external rig's config, both of which carry the key today), and
`orchestrator-loom/pipeline-on-queue-idle` receives the `# Tie-Off`
block from the next session on.

## Out of scope

- Per-knot override keys (there is no per-knot config file;
  unchanged).
- The enforcement parser (response-side parsing of the structured
  blocks — unchanged; the prompt-side guarantee is what this plan
  extends to leaf knots).
- `HANDOFF_NOTE` / `TASKS_INCOMPLETE_DESCRIPTION` / retry-note
  wording (still hard-coded literals — separate concern).
- The tie-off **file** shape the service writes (the `## <knot>
  triggered by …` header, `Timestamp:`, `session:`) — unchanged; the
  tie-off instruction governs only the agent's *response* content.
- Changing **where** the blocks sit relative to the knot's own
  instructions (both are prepended, as today — the order between the
  two new blocks is the only layout decision).
