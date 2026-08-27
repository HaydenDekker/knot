# Master Plan — Project Index

> **Last Updated:** 2026-08-27 (plan 080 complete — overflow without compaction fail-fast, released in v0.38.1)

## How to Add a Plan

Each plan file must contain a title (e.g. `# Plan: Plan Name`).

To add it to this index:

1. Add a row to the Master Progress Table: number, link, status, date.
2. Optionally add an overview section below with **goal** only. The goal states **what** the plan covers — not why or how. Full details belong in the plan file.

**Ordering:** Plans are ordered by creation date, latest first, within the table. Unknown dates (`—`) appear last.

---

## Purging Old Completed Plans

When updating `master-plan.md`, **remove any plan that is `✅ Complete` and meets this criteria:**

1. **Completed more than 4 weeks ago** — use completion date from the plan file's Implementation Status

Rationale: Once a plan has been complete for a significant period, its status in the index no longer provides active value. The plan file itself (in `project/plans/`) remains as historical documentation. Only the index entry is removed.

**What to remove:**
- The row from the **Master Progress Table**
- Any overview section for this plan

**What to keep:**
- The plan file in `project/plans/` — historical documentation
- **Do NOT renumber** — leave gaps in numbering to preserve historical references

**What NOT to remove:**
- Plans marked `🟡 In Progress`, `⬜ Planned`, or `❌ Blocked` — regardless of age
- Plans that are `✅ Complete` but completed within the last 4 weeks
- Plans with active dependencies (other plans that reference this one)

**What IS removed (after 4 weeks):**
- Plans marked `✅ Complete` and older than 4 weeks
- Plans marked `⬜ Planned (superseded by ...)` and older than 4 weeks — rationale should be captured in a design document

---

## Master Progress Table

| # | Plan | Status | Created |
|---|------|--------|---------|
| 80 | [Context Overflow Without Compaction — Fail Fast, Warn at Startup](080-overflow-error-fail-fast/overflow-error-fail-fast-plan.md) | ✅ Complete (2026-08-27) — released in v0.38.1 | 2026-08-27 |
| 79 | [Context Overflow — Compact and Continue](079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md) | ✅ Complete (2026-08-26) — released in v0.38.0 | 2026-08-26 |
| 78 | [Final-Response Request — Re-enter on Abrupt Turn-End](078-final-response-request/final-response-request-plan.md) | ✅ Complete | 2026-08-26 |
| 77 | [Empty Response Is Not a Timeout](077-empty-response-not-timeout/empty-response-not-timeout-plan.md) | ✅ Complete | 2026-08-25 |
| 76 | [Consumer Persistent Wake — No Lost Queue Notifications](076-consumer-persistent-wake/consumer-persistent-wake-plan.md) | ✅ Complete | 2026-08-25 |
| 75 | [Queue Entry Identity Self-Heal — Filename Is the Event ID](075-queue-identity-self-heal/queue-identity-self-heal-plan.md) | ✅ Complete | 2026-08-25 |
| 74 | [Thinking Level — Alias Default with Profile Override](074-thinking-level-hierarchy/thinking-level-hierarchy-plan.md) | ✅ Complete | 2026-08-24 |
| 73 | [Knot Step — Single-Event Stepping and Late Queue Removal](73-knot-step/knot-step-plan.md) | ✅ Complete | 2026-08-23 |
| 72 | [Clear Loom-Logs and Rig-Log at Startup](072-startup-log-clear/startup-log-clear-plan.md) | ✅ Complete | 2026-08-23 |
| 71 | [Record the Pi Session ID in Tie-Off Sections](071-tie-off-session-id/tie-off-session-id-plan.md) | 📝 Draft | 2026-08-22 |
| 70 | [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](070-dispatch-filename-collision/dispatch-filename-collision-plan.md) | ✅ Complete | 2026-08-22 |
| 69 | [Model Aliases — Rig-Level Model Registry](069-model-aliases/model-aliases-plan.md) | ✅ Complete | 2026-08-20 |
| 68 | [Rig Repository Separation](068-rig-repo-separation/rig-repo-separation-plan.md) | 📝 Draft | 2026-08-17 |
| 66 | [Relax the Knot Terminology Rule](066-terminology-rule-relaxation/terminology-rule-relaxation-plan.md) | 📝 Draft | 2026-07-29 |
| 63 | [Spurious Delete Suppression](063-spurious-delete-suppression/spurious-delete-suppression-plan.md) | 📝 Draft | 2026-07-16 |
| 31 | [Agent Profile Skills](agent-profile-skills.md) | ⬜ Planned | 2026-06-16 |
| 22 | [Notify Sender Leak Fix — Immediate Cascade Drain](notify-sender-leak-fix.md) | ⬜ Planned | 2026-06-11 |
| 20 | [Knot Modification Observability and Path Resolution Consistency](plan-knot-modify-observability.md) | 🟡 In Progress | 2026-06-08 |

---

_Overview sections for active and recently completed plans go here._

### 79. Context Overflow — Compact and Continue, with Loom-Log Visibility

**Status:** 📝 Draft
**Created:** 2026-08-26
**Goal:** Enable pi's built-in compaction for rig sessions (project-level `.pi/settings.json`, seeded by knot-init) so context overruns compact and continue inside pi instead of burning all session-resume retries; add a `ContextCompacted` loom-log entry per compaction so context pressure is visible for prompt-scoping; and fail fast on unrecoverable overflow (new non-resumable `PortError::ContextLimitReached`) so a context that cannot fit even after compaction stops immediately instead of clocking up 10 retries.

Full details in [079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md](079-context-overflow-compact-and-continue/context-overflow-compact-and-continue-plan.md).

### 78. Final-Response Request — Re-enter the Session When the Agent Stops Without a Final Response

**Status:** ✅ Complete (2026-08-26) — released in v0.37.2
**Goal:** When the agent ends its turn without a final response and a session ID was captured, re-enter the session (`--session-id`) with the final-response request — *"Please produce your final response, or continue if you have not finished."* — through the existing session-resume retry loop, so an abrupt stop is nudged once per attempt (bounded by the profile timeout budget) instead of failing immediately.

Full details in [078-final-response-request/final-response-request-plan.md](078-final-response-request/final-response-request-plan.md).

### 77. Empty Response Is Not a Timeout

**Status:** ✅ Complete (2026-08-26) — released in v0.37.1
**Goal:** Make an abrupt turn-end (agent exits 0 with no final response) a first-class failure instead of a spurious timeout — a new `PortError::AgentNoResponse` (resumable, carries the session ID for plan 078) is produced by both empty-response paths in session resume, and `TieOffOutcome::derive` maps it to a failed tie-off so the rig-log's `TimeoutExceeded` records genuine deadline breaches only.

### 76. Consumer Persistent Wake — No Lost Queue Notifications

**Status:** ✅ Complete (2026-08-25) — released in v0.37.0 (combined with plan 075)
**Goal:** Make the queue wake-up persistent — `StrandEventQueue::notified()` arms its `Notify` permit at call time and the consumer loops (service + step) arm it before re-checking `front()` — so a push can never be missed while the loop is idle and the "queue idle with a non-empty queue, no log" symptom class is closed structurally.

### 75. Queue Entry Identity Self-Heal — Filename Is the Event ID

**Status:** ✅ Complete (2026-08-25) — released in v0.37.0 (combined with plan 076)
**Goal:** Make the disk event queue self-heal the filename-stem ⇄ JSON-`id` invariant on every scan (filename wins, atomic repair, warning logged) and make `front()`/`pop()`/dedup/late-removal operate on the healed id, so a renamed (e.g. backdated) queue file reorders the FIFO as intended instead of wedging the pipeline with a phantom head — the root cause of the 2026-08-25 borrow-my-stuff idle-with-nonempty-queue incidents.

### 74. Thinking Level — Alias Default with Profile Override

**Status:** ✅ Complete (2026-08-24)
**Created:** 2026-08-24
**Goal:** Add an optional `thinking-level` (`off|minimal|low|medium|high|xhigh`) to `rig/models.yml` aliases as a per-model default and to profile frontmatter as an override (profile takes precedence), resolved into `AgentConfig` and emitted as `--thinking <level>` on the pi invocation, with the effective level visible in `state.json`.

Completed in Knot 0.36.0: `ThinkingLevel` value object (lexical validation, exact-token `Display`); optional `thinking-level` on `models.yml` aliases and profile frontmatter (invalid values rejected at parse — registry degrades to warning + empty registry, profile is a hard error); `profile.or(alias)` hierarchy in `resolve_for_knot` (direct-spec profiles use their own level only, registry unconsulted); `--thinking <level>` emitted for every effective value including explicit `off` (omission emits no flag — pi's settings default applies); effective level on `state.json` profile entries (omitted, never null, when unset); acceptance tests through both pi runners with argv capture. No document migration. Design knowledge in [design/design-thinking-level.md](../design/design-thinking-level.md).

Full details in [074-thinking-level-hierarchy/thinking-level-hierarchy-plan.md](074-thinking-level-hierarchy/thinking-level-hierarchy-plan.md).

### 73. Knot Step — Single-Event Stepping and Late Queue Removal

**Status:** ✅ Complete (2026-08-23)
**Created:** 2026-08-23
**Goal:** Add a `knot step` CLI command that processes exactly one queued event and exits (full startup, single execution, graceful shutdown), and change the disk-backed queue to late removal (at-least-once) so a crash mid-processing re-queues the event instead of losing it.

Completed in Knot 0.35.0: `front()`/`shutdown_signaled()` on the queue port; `ProcessStrand::execute_with_pending` with removal just-before-commit on success and at the point of failure otherwise (exactly-once-removal invariant); front-based service loop; pure unit-tested `parse_args` with `step` (stricter rig discovery — no implicit `rig/`); `step_knot` lifecycle (`StartupOptions` split, shared `build_process_strand`, `--event` resolution, 5× debounce empty-queue wait, in-cycle settle, service-identical shutdown cascade; in-cycle events captured but not executed). 20 new late-removal tests, 19 new step tests (lib + binary-level), 10 parse-args unit tests. Design knowledge in [design/design-knot-step.md](../design/design-knot-step.md).

Full details in [73-knot-step/knot-step-plan.md](73-knot-step/knot-step-plan.md).

### 72. Clear Loom-Logs and Rig-Log at Startup

**Status:** ✅ Complete (2026-08-23)
**Created:** 2026-08-23
**Goal:** Truncate the rig-log and every loom-log at knot startup (after legacy migration, before discovery) so each run's logs contain only current-run events — removing unbounded cross-run growth and the repeated `WARN:` console spam from stale unparseable lines, while tie-offs remain the durable audit history.

**Outcome:** `RigLogPort::clear()` / `LoomLogPort::clear_all()` (required trait methods) truncate in place; `run_startup` runs them after legacy migration, before discovery — non-fatal on error. Orphaned loom dirs are cleared too; only log files are touched (tie-offs, dispatch dirs, `state.json`, `events/` untouched). Bumped to 0.34.0. Design reference: `project/design/design-startup-log-clear.md`.

Full details in [072-startup-log-clear/startup-log-clear-plan.md](072-startup-log-clear/startup-log-clear-plan.md).

### 71. Record the Pi Session ID in Tie-Off Sections

**Status:** 📝 Draft
**Created:** 2026-08-22
**Goal:** Record the pi session ID captured during agent execution as an optional `session:` metadata line in tie-off sections, so every tie-off section is traceable to the exact pi session that produced it.

Full details in [071-tie-off-session-id/tie-off-session-id-plan.md](071-tie-off-session-id/tie-off-session-id-plan.md).

### 70. Unique Event Dispatch Filenames — Per-Batch Sequence Suffix

**Status:** ✅ Complete (2026-08-22)
**Created:** 2026-08-22
**Goal:** Make event dispatch filenames unique per dispatch batch — same-second fan-out of one event type to one consumer gets per-directory sequence suffixes (`event-{ts}-001.md` …) and atomic `create_new` file creation, so no two dispatches can target one path.

Completed in Knot 0.33.0: pure `event_file_name(ts, seq)` helper; `seq: u32` on `EventDispatcherPort::dispatch` with use-case batch sequencing (singleton → plain name, N > 1 → `-001…-NNN` in emission order); atomic creation with bounded taken-name fallback; `EventsDispatched` loom-log entries carry the created file path (legacy 3-tuple lines skip with a warning); end-to-end acceptance test replaying the 2026-08-22 four-way `ValidationFail` incident. Design knowledge in [design/design-event-dispatch.md](../design/design-event-dispatch.md).

Full details in [070-dispatch-filename-collision/dispatch-filename-collision-plan.md](070-dispatch-filename-collision/dispatch-filename-collision-plan.md).

### 69. Model Aliases — Rig-Level Model Registry

**Status:** ✅ Complete (2026-08-20)
**Created:** 2026-08-20
**Goal:** Add a rig-level model registry (`rig/models.yml`) mapping aliases to `{provider, model}` pairs, plus an optional `model-ref` field on profiles (alias takes highest priority over direct spec), so swapping a model is a one-line edit picked up live on the next strand — no per-profile edits, no restart.

Completed in Knot 0.32.0: `ModelRegistry` domain value, `ModelRegistryPort` + `FileSystemModelRegistry` (fresh read per resolution — live swap), `model-ref` profile frontmatter with alias-over-direct precedence, state visibility (resolved provider/model, null when unresolvable), `run_startup` auto-creates `rig/models.yml`, and updated skills (knot-update/create/inspect/init).

Full details in [069-model-aliases/model-aliases-plan.md](069-model-aliases/model-aliases-plan.md).

### 63. Spurious Delete Suppression

**Status:** 📝 Draft
**Created:** 2026-07-16
**Goal:** Suppress spurious DELETE events caused by atomic file rewrites (truncate+write) using a configurable 5-second suppression window in the debounce engine.

Full details in [063-spurious-delete-suppression/spurious-delete-suppression-plan.md](063-spurious-delete-suppression/spurious-delete-suppression-plan.md).

### 31. Agent Profile Skills

**Status:** ⬜ Planned
**Created:** 2026-06-16
**Goal:** Add `skills` field to agent profiles so Knot passes `--no-skills` + `--skill <path>` to `pi`, making the agent's skill set explicit and keeping context concise.

**PRD:** [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md)

Full details in [agent-profile-skills.md](agent-profile-skills.md).

### 22. Notify Sender Leak Fix — Immediate Cascade Drain

**Status:** ⬜ Planned
**Created:** 2026-06-11
**Goal:** Split `NotifyEventSource` senders from callback state so channels close immediately on drop, removing the 5-second timeout safety net.

Full details in [notify-sender-leak-fix.md](notify-sender-leak-fix.md).

### 20. Knot Modification Observability and Path Resolution Consistency

**Status:** 🟡 In Progress
**Created:** 2026-06-08
**Completed (Phase 0):** 2026-06-15
**Goal:** Make `KnotModified` filesystem changes observable via loom-log (`LoomEvent::KnotUpdated`), log parse failures to stderr, and ensure path resolution is consistent between initial load and file-watcher events.

**Result (Phase 0):** `NotifyEventSource` now receives correct `project_root` (parent of rig directory) so relative `strand_dir` paths resolve identically to `FileSystemLoomRepository::scan()`. Full rename `base_dir` → `rig_dir` across all 7 source files + 17 test files to eliminate ambiguity between "rig directory" and "project root". Remaining phases (KnotUpdated loom-log, parse failure logging, integration test) still pending.

Full details in [plan-knot-modify-observability.md](plan-knot-modify-observability.md).
