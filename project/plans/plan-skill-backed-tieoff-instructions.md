# Plan: Skill-backed tie-off and event-content prompt injection

**Status:** ✅ Complete (2026-10-16) — released in v0.59.0 (design rev 2)
pre-roll-out. Phases R0–R5 (below) supersede the `rig/skills/` parts of
the original design; the original phases 0–5 were implemented on
2026-10-16 and remain the record of what was built.
**Depends on / sibling to:**
`plan-bugfix-event-enforcement-followup-tieoff.md` (the follow-up re-injection
automatically inherits whatever the listener context contains — no change
needed there).

## Problem

The tie-off instruction delivered to executing agents is hard-coded in Rust
string literals (`build_listener_context` in `src/domain/events.rs`), as is
the guidance for how to fill in an event block's `description`, body, and
extra fields. Consequences (established in the audit):

- The contract can only change with a Knot binary release — no per-rig or
  per-project tuning.
- The wording has small quality defects baked in (typos: "acknolowedge",
  "pendening", "occured") — inside the very contract that
  event-enforcement tests the agent against.
- There is no single document a user can read, edit, version, or point at.

## Target

> ⚠️ **Superseded in part** — the `rig/skills/` default-file design below
> was revised pre-roll-out; see the **Design revision (2026-10-16)**
> section. What remains valid from this section: the two instruction
> skills and what each owns, the two override keys, and the per-session
> pull semantics.

Two **instruction skills** — plain markdown files that Knot manages and
injects into the prompt, in place of the hard-coded text:

| Skill | Default file | Owns |
|---|---|---|
| `knot-tie-off` | `rig/skills/knot-tie-off.md` | The tie-off contract: the final response *is* the tie-off; one structured event block per subscriber event; frontmatter format; `occurred` semantics; `event: None` blanket acknowledgement; the "never read/write `tie-offs/` directly" rule |
| `knot-event-content` | `rig/skills/knot-event-content.md` | How to write each block's content: the `description` field (why the event did/didn't occur + requested info), the narrative body, and the extra fields a subscriber declares in its `event_description` |

**Override mechanism** — two new optional keys in
`rig/.workspace-agent-config.yaml`:

```yaml
# Optional full replacement for the corresponding default skill file.
# If the file exists and is readable, Knot injects its FULL content in
# place of the default. Relative paths resolve against the rig directory.
tie-off-instruction-file: path/to/my-tieoff.md
event-content-instruction-file: path/to/my-event-content.md
```

- Both keys absent (or file missing/unreadable) → the default skill file is
  used. The user therefore has exactly two escape hatches: **edit the
  default skill file directly** (it lives in the rig and is
  git-versioned) or **point the key at their own file**.
- **Pulled per new session**: the file (and the config key) is re-read at
  every new agent session — i.e. per strand execution — never cached from
  startup and never from an earlier run. Edit the file mid-run and the next
  session sees it. (An enforcement follow-up re-enters the *same* session,
  so it reuses that session's already-injected context — unchanged.)

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Where the default skills live | `rig/skills/knot-tie-off.md`, `rig/skills/knot-event-content.md` (flat files) | Rig-scoped, git-versioned with the rig, trivially injectable ("inject the full file"). Not pi skills — the executing agent receives the content in its prompt; no skill discovery needed in the executing session |
| "Managed by knot" | `run_startup` (server.rs:964) **seeds both files from embedded defaults if missing and never overwrites existing files** — same pattern as the existing auto-creation of `.workspace-agent-config.yaml` (server.rs:977) and `models.yml` (server.rs:995) | The binary owns the default wording (single source of truth); user edits win after the first seed |
| Override key scope | Rig-level (the existing `.workspace-agent-config.yaml` is per-rig; there is no per-knot config file) | Matches the existing file. Per-knot overrides: non-goal (see below) |
| Override value | Local file path: absolute, or relative to the rig directory; a `file://` URL is accepted and stripped | Knot is local-first with no network listener — `http(s)://` is **refused** (falls back to the default with a warning), no network fetch ever |
| Fetch timing | Per new session, inside prompt construction; no cache anywhere (config keys and skill file both re-read each time) | "The skill has to be pulled each new session"; also means a new override key takes effect **without restarting Knot** (deliberate deviation from the start-restart convention of the existing keys — the cost is two tiny file reads per session) |
| Fallback on missing/unreadable file | Embedded default (the current hard-coded text, split per the table above) + one `[WARN]` service-log line | Old rigs (no `rig/skills/` yet, e.g. before the seeding version restarts) keep working byte-compatibly; a typo in an override path must never blank the contract |
| Dynamic vs static split | The code keeps building **only the dynamic subscriber list** (event id + per-subscriber description, which `extract_emitted_event_ids` parses — format unchanged); the `# Subscriber Events` intro, the `## Event Format` example, and the `## Rules` text move into the two default skills | The worked example currently interpolates the first real event id — that generation is dropped in favour of the skill's generic example; everything else about the block format the agent is enforced against is static wording and belongs in the skill |
| Split line between the two skills | Mechanics (what/when/shape) → `knot-tie-off`; filling-in (description, body, extra fields) → `knot-event-content` | Matches the two user goals: "the tie-off message" vs "how to write the event description and body content and any custom properties" |
| Typos | Fixed while splitting ("acknolowedge", "pendening", "occured") | The enforcement follow-up literally names the contract back at the agent; the wording should be clean |
| Non-goals | `HANDOFF_NOTE` / `TASKS_INCOMPLETE_DESCRIPTION` / retry notes stay hard-coded (separate plan if wanted); no-subscriber baseline injection (currently: no injection at all when no listeners — a separate plan); per-knot override keys | Keeps this change a pure "where the two instruction texts come from" swap |

## Design revision (2026-10-16, rev 2) — supersedes Target, Decisions, and rev 1

After v0.59.0 was implemented but before rollout, the user revised the
design twice. Rev 1 (same day) moved the defaults out of the rig into
loose template files committed next to the knot skills. **Rev 2 (this
section) wraps those templates in a pi skill** — `knot-template` — so the
customisation mechanism is agent-readable and the project override is
created by invoking the skill.

**The design (four parts):**

1. **Hard-coded default** — the embedded text
   (`DEFAULT_TIE_OFF_INSTRUCTION` / `DEFAULT_EVENT_CONTENT_INSTRUCTION` in
   `src/domain/events.rs`) is the runtime default, always available. It is
   compiled in via `include_str!` from the template files that live
   **inside the `knot-template` skill directory** — the repo files and the
   binary's default cannot drift apart.
2. **The `knot-template` skill** — a pi skill
   (`.agents/skills/knot-template/SKILL.md`, deployed to
   `~/.agents/skills-library/knot-template/` with the other knot
   sub-skills, routed from the `knot` master) that:
   - explains the instruction mechanism (the hard-coded default, the two
     config keys, the per-session pull, the resolution chain), and
   - **provides the templates** — `knot-tie-off.md` +
     `knot-event-content.md` alongside the SKILL.md (the
     `knot-init/knot-glossary.md` companion-file pattern).
3. **Agent-driven override creation** — the user invokes the skill
   ("set up a custom tie-off instruction"); the agent follows SKILL.md:
   copies the two templates from the skill directory into the project at
   `rig/knot-template/knot-tie-off.md` +
   `rig/knot-template/knot-event-content.md`, and adds the two keys to
   `rig/.workspace-agent-config.yaml` (merge, never clobber). The project
   files then override the hard-coded default for that rig.
4. **Nothing is ever written into the rig by the service** — Knot reads
   the config link (if any) or the embedded default; it never creates
   files. The skill (the agent) is the only writer.

**Runtime resolution (per new session):** config link (readable file) →
hard-coded embedded default.

### Revised decisions (supersede the matching rows above and rev 1)

| Decision | Choice | Rationale |
|---|---|---|
| Where the default lives | **In the binary** (embedded via `include_str!` of the template files in `.agents/skills/knot-template/`) — never in the rig | The default is a property of the Knot version, not of a rig: identical on every machine, versioned with `knot --version`, no drift |
| Template files | `.agents/skills/knot-template/knot-tie-off.md` + `knot-event-content.md` — the **single source of truth** (compiled into the binary via `include_str!` and shipped with the skill) | One file serves both roles: the binary's hard-coded default and the skill's starting point for project overrides |
| The skill | `knot-template` — a pi-discoverable skill (SKILL.md with frontmatter) deployed as a knot sub-skill to `~/.agents/skills-library/knot-template/` and routed from the `knot` master | Agent-readable procedure: the user asks, the agent knows exactly how to wire the override. (True auto-discovery into every system prompt would require a master in `~/.agents/skills/` — deliberately not done: it would pollute every session; master routing is the ecosystem convention) |
| Override creation | **Invoking the skill** — the agent copies the templates into `rig/knot-template/` and sets the config links (merging into the existing config) | "Have a template added for the project": the project owns the override files, git-versioned with the rig |
| Override location | `rig/knot-template/knot-tie-off.md` + `knot-event-content.md`; config values `knot-template/knot-tie-off.md` / `knot-template/knot-event-content.md` (relative to the rig dir) | Simple relative links; the override travels with the rig |
| Override value | File reference ("url"): absolute path, relative to the rig dir, or `file://` URL; leading `~/` expanded to `$HOME`; `http(s)://` **refused** (local-first; a separate decision if remote fetching is ever wanted) | Knot never fetches over the network |
| Fallback | Linked file missing / unreadable → warning + hard-coded embedded default | A typo'd link must never blank the contract |
| Already-seeded rigs | Rigs that ran the pre-revision 0.59.0 build have inert `rig/skills/knot-*.md` files (never read at runtime) — safe to delete; recorded in `knot-update` | No migration |

### Work in flight (at the moment of this revision)

- [x] `.agents/skills/knot-tie-off.md` + `.agents/skills/knot-event-content.md`
      created (rev 1) — verified byte-identical to the previously embedded
      text (extraction check) — **to be moved** into
      `.agents/skills/knot-template/`
- [x] `src/domain/events.rs` — both `DEFAULT_*` constants are now
      `include_str!` of those files (the paths need the one-line update to
      the new location)
- [x] Doc comments in `events.rs` still describe the superseded seeding
      design
- [x] The adapter still resolves a `rig/skills/` fallback tier
      (superseded)
- [x] The `server.rs` `run_startup` seed block + its two tests
      (superseded)
- [x] `.agents/skills/knot-template/SKILL.md` — not yet written

### Phase R0: the `knot-template` skill (repo)

**Files:** `.agents/skills/knot-template/` (new dir), `src/domain/events.rs`,
`.agents/skills/knot/SKILL.md`

- [x] Move the two template files into `.agents/skills/knot-template/`
      (delete the loose copies from `.agents/skills/`)
- [x] Write `knot-template/SKILL.md`: frontmatter (name + a
      USE FOR / DO NOT USE FOR description covering "customise knot
      instructions, tie-off instruction, event content, add knot template,
      knot config"); body: the mechanism (hard-coded default → config link
      → per-session pull), the procedure to add a project override (copy
      the templates from this skill directory to `rig/knot-template/`,
      merge the two keys into `rig/.workspace-agent-config.yaml` — never
      clobber the existing keys), and pointers to the two template files
      (relative paths resolve against the skill directory)
- [x] `src/domain/events.rs` — update the `include_str!` paths to
      `../../.agents/skills/knot-template/knot-*.md`; update the doc
      comments (constants, trait, `BuildContext.instruction_source`) to
      the final design
- [x] The `knot` master skill — routing-table row for `knot-template`

### Phase R1: adapter — drop the `rig/skills/` tier

**Files:** `src/adapters/outbound/prompt_instruction_source.rs`

- [x] Remove the `SKILLS_DIR` / default-skill-file tier; resolution
      becomes: per-rig config link (readable file → `Some(content)`) →
      `None` (the domain then uses the embedded default)
- [x] Expand a leading `~/` in override values to `$HOME`
- [x] Keep: fresh read per call (config + linked file); `file://`
      stripping; `http(s)` refusal with a warning; malformed-config
      warning
- [x] Tests: relative / absolute / `file://` / `~/` links honoured; link
      missing → `None` + warning; `http(s)` refused → `None`; malformed
      config → `None`; re-pull per call; keys do not leak across each
      other

### Phase R2: remove the startup seeding

**Files:** `src/server.rs`

- [x] Delete the `rig/skills` seeding block from `run_startup` (+ the
      now-unused constant imports)
- [x] Delete `test_startup_seeds_instruction_skill_files` and
      `test_startup_preserves_edited_instruction_skill_files`
- [x] Note in the `knot-update` entry: pre-revision 0.59.0 rigs may have
      inert `rig/skills/knot-*.md` files — safe to delete

### Phase R3: docs, skills, and the deploy flow

- [x] `AGENTS.md` — add `knot-template` to the sub-skill deploy loop
      (the whole-dir copy carries the two template files; extend the diff
      verification) and a short note on what `knot-template` is (the
      hard-coded default's source of truth; invoked to add a project
      override)
- [x] `docs/concepts.md` — rewrite the skill-backed paragraph: hard-coded
      default, the `knot-template` skill, the config link, per-session
      pull, nothing written to the rig by the service
- [x] `docs/configuration/rig-structure.md` — drop the `skills/` line
      from the rig tree; rewrite the overrides section: the keys link a
      full replacement (absolute / relative / `file://` / `~/`); the
      `knot-template` skill creates `rig/knot-template/` + the links
- [x] `docs/release-notes.md` — rewrite the (unreleased) v0.59.0 entry to
      match the final design
- [x] `.agents/skills/knot-update/SKILL.md` — rewrite the 0.59.0 entry
      (skill version 1.25.0): hard-coded default, the new `knot-template`
      skill, the two optional config keys, the enforcement follow-up
      tie-off fix; migration: none (inert `rig/skills/` files from a
      pre-revision 0.59.0 build may be deleted)

### Phase R4: verification

- [x] `cargo test` (full suite)
- [x] `cargo clippy` — no new warnings

### Phase R5: deploy + install

- [x] Deploy `knot-template` (the whole directory, including the two
      template files) + the updated `knot` master + `knot-update` to
      `~/.agents`, diff-verify, and commit in the `~/.agents` repo
- [x] `cargo install --path . --locked` (stays 0.59.0 — a pre-rollout
      design revision, not a new version)
- [x] `knot --version` reports 0.59.0

Per AGENTS.md: **no live rig runs in this repo** — verification via
`cargo test` / `cargo clippy` and the mock-CLI harness only.
## Changes (implemented 2026-10-16; parts superseded by the phases above)

### Phase 0: Default skill content + embedded constants

**Files:** `src/domain/value_objects.rs` (or `src/domain/events.rs` — keep
co-located with the current text it replaces), `rig/skills/*.md` (seeded
at runtime, committed as seed fixtures in tests)

- [x] Extract the current `build_listener_context` wording into two
      embedded constants:
      `DEFAULT_TIE_OFF_INSTRUCTION` and `DEFAULT_EVENT_CONTENT_INSTRUCTION`
      (the split per the table; typos fixed)
- [x] Draft `rig/skills/knot-tie-off.md` and `rig/skills/knot-event-content.md`
      content (SKILL-style markdown: name/description header + the contract
      sections) — these are the seed bodies; on first review the constants
      and the committed skill files must match

### Phase 1: `PromptInstructionSource` port + filesystem adapter

**Files:** `src/application/ports.rs`, `src/adapters/outbound/` (new
`prompt_instruction_source.rs`)

- [x] Port:
      ```rust
      /// Reads per-session instruction skill content. Every call is a
      /// fresh read (no caching) — the skill is pulled each new session.
      pub trait PromptInstructionSource: Send + Sync {
          /// `None` → caller falls back to the embedded default.
          fn read_tie_off_instruction(&self) -> Option<String>;
          fn read_event_content_instruction(&self) -> Option<String>;
      }
      ```
- [x] `FileSystemPromptInstructionSource { rig_dir: PathBuf }`: per call —
      1. read `.workspace-agent-config.yaml` (parse into `RigAgentConfig`
         with the two new optional keys, `Option<String>`, serde-defaulted
         to `None` so existing files are unchanged);
      2. if the key is `Some(path)` → resolve (absolute, relative-to-rig,
         or `file://`); `http(s)://` → warn + treat as absent; read the
         file; unreadable → warn + fall through;
      3. otherwise read the default `rig/skills/<skill>.md`;
         missing/unreadable → `None` (embedded fallback) with a warn for
         the unreadable case only (missing is expected pre-seed).
- [x] New `RigAgentConfig` fields:
      `tie_off_instruction_file: Option<String>`
      (`tie-off-instruction-file`) and `event_content_instruction_file`
      (`event-content-instruction-file`), serde-defaulted `None`,
      kebab-case, with doc comments matching the file header format
- [x] Adapter unit tests: default read; override honored; override missing
      → default; override path invalid / `http(s)` refused → default +
      warning; relative path resolution; **re-pull per call** (modify file
      between calls → second call reflects it); legacy config without the
      keys parses unchanged

### Phase 2: Injection in the prompt pipeline

**Files:** `src/domain/events.rs` (`build_listener_context`,
`BuildContext`), `src/application/usecases/context_providers.rs`
(`AgentEventsContextProvider`), `src/application/usecases/process_strand.rs`,
`src/server.rs` (composition root)

- [x] `build_listener_context` gains two parameters
      `tie_off_instruction: Option<&str>` and
      `event_content_instruction: Option<&str>`; when `Some`, the file
      content is injected under `## Tie-Off Contract` /
      `## Event Content Guide` **in place of** the corresponding embedded
      text; when `None`, the embedded default is used (today's output,
      modulo typos). The dynamic subscriber list and pending-events
      sections are untouched; the interpolated worked example is dropped
      (the skill carries the generic example)
- [x] `AgentEventsContextProvider` becomes a struct holding
      `Arc<dyn PromptInstructionSource>`; `build_full_context` fetches both
      texts per call and passes them through (domain stays pure — no fs in
      the domain layer)
- [x] Composition root (`build_app_context`, server.rs) constructs
      `FileSystemPromptInstructionSource::new(rig_dir)` and wires it into
      the provider; `ProcessStrand`'s `BuildContext` construction is
      unchanged (it already carries `rig_dir` for the pending-event scan)
- [x] Enforcement follow-up (`inject_event_request`): no change — it
      re-attaches the run's `listener_context`, which now carries the
      skill sections automatically
- [x] Tests: domain-level — `Some` content replaces the default block
      (assert skill text present, default "## Rules" text absent); `None`
      → default (regression, current snapshot); the subscriber-list bullet
      format is byte-identical (protects `extract_emitted_event_ids`);
      application-level — a counting mock source proves one fetch per
      session (two consecutive `process_strand` executions → two reads)
      and that a file edit between executions changes the second prompt

### Phase 3: Startup seeding ("managed by knot")

**File:** `src/server.rs` (`run_startup`)

- [x] After the existing config-file / `models.yml` auto-creation: seed
      `rig/skills/knot-tie-off.md` and `rig/skills/knot-event-content.md`
      from the embedded constants **if and only if the file is missing**;
      never overwrite (user edits are durable)
- [x] Update `rig/.workspace-agent-config.yaml` header comment with the
      two new keys (documentation, matching the existing style)
- [x] Tests: seed creates both files with the constant content; existing
      files (modified) are left byte-identical after startup

### Phase 4: Docs, skills, release notes

- [x] `docs/concepts.md` — new short section "Prompt instruction skills":
      the two files, the override keys, the per-session pull, the
      seed-if-missing rule
- [x] `docs/release-notes.md` — entry: instruction skills extracted from
      the binary; override keys; seeded files; typo fixes in the
      subscriber-events wording
- [x] `.agents/skills/knot-init/SKILL.md` — note the new
      `rig/skills/` files (informational; the binary seeds them at
      startup, init does not need to)
- [x] `.agents/skills/knot-update/SKILL.md` — changelog entry for the new
      version: new rig files (`rig/skills/*`), two optional config keys
      (additive, no migration), changed prompt wording (typos)
- [x] `project/domain-glossary.md` — add "instruction skill" /
      "tie-off instruction" if the glossary warrants it

### Phase 5: Verification

- [x] `cargo test` (full suite)
- [x] `cargo clippy` — no new warnings
- [x] `git diff` review of the seeded skill content vs the embedded
      constants (must match at seed time)

Per AGENTS.md: **no live rig runs in this repo** — verification via
`cargo test` / `cargo clippy` and the mock-CLI harness only.

## Risks / notes

- **Enforcement coupling** — event enforcement
  (`missing_event_ids` / `has_no_events`) parses the agent's *response*,
  not the prompt, so a user override that changes the block format only
  breaks enforcement if the agent follows the override's format. That is
  the user's choice; the plan documents it, and the warning path
  (missing override file → default) keeps the common case safe.
- **`extract_emitted_event_ids` parses the injected bullet list**
  (`- \`EventId\` — ...`) — the Phase 2 byte-identity assertion is the
  guard against an accidental format drift.
- **Prompt size** — the two skills add roughly the same bytes the current
  literals add; no meaningful change. An override *should* be of similar
  size (it is a replacement, not an addition).
- **Pre-seed rigs** — a rig created before this version has no
  `rig/skills/` files until the first restart; the `None` → embedded
  default fallback keeps prompt output identical in the meantime.
- Precedent: `load_rig_config`'s warn-and-fallback posture
  (server.rs:153–164) is the template for all failure paths here.
