# Master Plan — Project Index

> **Last Updated:** 2026-09-12 (plan 090 complete — concise in-session retry prompts: a `--session-id` re-entry sends the cause-specific note only, with no re-send of the original prompt, released in v0.48.0)
> **Prior:** 2026-09-12 (plan 089 complete — follow-on phases 8–13 shipped in v0.47.0: settle-based teardown, in-session continuation after a compaction, any-reason interruptions, compaction-aware inactivity window, `TurnContinued` / `RunAbandoned`)
> **Prior:** 2026-09-12 (plan 089 reopened from the 2026-09-11/12 `pwa-todo-3` evidence — threshold compactions still ended the attempt and compaction stalls tripped the inactivity watchdog)
> **Prior:** 2026-09-11 (plan 089 phases 0–7 complete — Interrupted Overflow Compaction: manual compact + session-restart recovery when pi's in-process recovery dies mid-compaction, released in v0.46.0)

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
| 91 | [Per-Event Enforcement — Re-Ask When a Tie-Off Acknowledges Only Some Expected Events](091-per-event-enforcement/per-event-enforcement-plan.md) | ✅ Complete (2026-09-20) — released in v0.49.0 | 2026-09-20 |
| 90 | [Concise In-Session Retry — Stop Re-Sending the Original Prompt on Session Re-Entry](090-concise-in-session-retry/concise-in-session-retry-plan.md) | ✅ Complete (2026-09-12) — released in v0.48.0 | 2026-09-12 |
| 89 | [Interrupted Overflow Compaction — Manual Compact and Session-Restart Recovery](089-interrupted-compact-manual-recovery/interrupted-compact-manual-recovery-plan.md) | ✅ Complete (2026-09-12) — phases 0–7 released in v0.46.0, phases 8–13 in v0.47.0 | 2026-09-11 |
| 88 | [Compaction Assurance — Auto-Compaction Always On, Overflow Recovery Continues the Session](088-compaction-assurance/compaction-assurance-plan.md) | ✅ Complete (2026-09-11) — released in v0.45.0 | 2026-09-11 |
| 87 | [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget/087-event-source-continuation-and-queue-budget-plan.md) | ✅ Complete (2026-09-10) — released in v0.44.0 | 2026-09-10 |
| 86 | [Graceful Task Handoff — Checkpointed Continuation Chains for Task-Bearing Knots](086-graceful-task-handoff/graceful-task-handoff-plan.md) | ✅ Complete (2026-09-08) — released in v0.43.0 | 2026-09-08 |
| 85 | [Event-Parse Log Flags and Anchored Rig `.gitignore` Entry](085-event-log-flags-and-anchored-gitignore/085-event-log-flags-and-anchored-gitignore-plan.md) | ✅ Complete (2026-09-07) — released in v0.41.1 | 2026-09-07 |
| 84 | [Graceful Completion — Steer the Session to Wrap Up Before Context Runs Out](084-graceful-completion/graceful-completion-plan.md) | ✅ Complete (2026-09-07) — released in v0.42.0 | 2026-09-07 |
| 83 | [Consolidated Service Log + Change-Driven State Writes](083-consolidated-service-log/consolidated-service-log-plan.md) | ✅ Complete (2026-09-07) — released in v0.41.0 | 2026-09-07 |
| 82 | [System Event Subscriptions — Strand Off Any Knot Event, with Wildcard Producers](082-system-event-subscriptions/system-event-subscriptions-plan.md) | ✅ Complete (2026-09-07) — released in v0.41.0 | 2026-09-07 |
| 81 | [Inactivity Timeout — Kill Blocked Sessions, Restart with a Blocking-Call Note](081-inactivity-timeout/inactivity-timeout-plan.md) | ✅ Complete (2026-09-03) — released in v0.40.0 | 2026-09-02 |
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

### 90. Concise In-Session Retry — Stop Re-Sending the Original Prompt on Session Re-Entry

**Status:** ✅ Complete (2026-09-12) — released in v0.48.0
**Created:** 2026-09-12
**Goal:** Key the session-resume retry prompt shape on session re-entry: an in-session retry (`--session-id`) sends the cause-specific note **only** — the final-response request by default, the inactivity restart note, the compaction restart note, or the water-mark handoff note — with an empty profile prompt and no cross-attempt accumulation (the session already holds the persona and the original prompt); a fresh restart (inactivity stall before the session ID was captured) keeps the full composed prompt plus the note, since a fresh process has no history.

Completed in Knot 0.48.0: the retry loop in `src/application/session_resume.rs` keys the prompt shape on `session_id` (in-session → note + empty profile prompt; fresh → full prompt + note, byte-for-byte as before); the `@strand-file` attachment stays on every attempt; bounds, budget math, events, and terminal-error classification unchanged. Reverses plan 078's "why re-send the original prompt" decision now that the codebase has two proven concise in-session shapes (`inject_event_request`, the D6 live continuation). No rig-document migration (knot-update carries no entry).

Full details in [090-concise-in-session-retry/concise-in-session-retry-plan.md](090-concise-in-session-retry/concise-in-session-retry-plan.md).

### 89. Interrupted Overflow Compaction — Manual Compact and Session-Restart Recovery

**Status:** ✅ Complete (2026-09-12) — phases 0–7 released in v0.46.0, phases 8–13 in v0.47.0
**Created:** 2026-09-11
**Goal:** When pi's in-process overflow compact-and-retry dies mid-compaction (the process stops after `compaction_start` and before `compaction_end`), recover the session instead of failing it: detect the interrupted compaction, run a manual `compact` on the same session via the `pi-rpc` adapter, and re-enter with `--session-id` — logging each boundary (`CompactionInterrupted`, `ManualCompactionSucceeded` / `ManualCompactionFailed`, `SessionRestarted`) — and stop the adapter from killing a live in-flight compaction.

**Follow-on goal (phases 8–13):** finish the teardown fix (tear down on pi's `agent_settled`, and never close stdin while a compaction span is open — pi exits on stdin EOF, which is why every threshold compaction still ends its attempt), ask the compacted session to continue **in the same process** instead of restarting it, treat an interrupted `threshold` compaction as the interruption it is, let a compaction span count as activity for the inactivity watchdog, and make the remaining edges visible (`session=` on resumed attempts, abandoned in-flight runs).

Full details in [089-interrupted-compact-manual-recovery/interrupted-compact-manual-recovery-plan.md](089-interrupted-compact-manual-recovery/interrupted-compact-manual-recovery-plan.md).

### 88. Compaction Assurance — Auto-Compaction Always On, Overflow Recovery Continues the Session

**Status:** ✅ Complete (2026-09-11) — released in v0.45.0
**Created:** 2026-09-11
**Goal:** Close the four gaps in the rig's compaction story without adding any runtime mechanism (overflow recovery stays pi's; Knot's role stays observe + fail fast): (1) auto-compaction is **always on** for rig sessions — the service self-heals the project `.pi/settings.json` at startup (merges `compaction.enabled: true`, never clobbers, never touches the global file, honours and warns an explicit project-level opt-out); (2) it fires on context overflow via pi's in-process compact-and-retry; (3) the session continues after compaction — within the run and across session-resume re-entry with the captured session id (Knot's half pinned by test); and (4) `compaction_start` / `compaction_end` surface as **live** loom events — `CompactionStarted` / `ContextCompacted` / `ContextCompactionFailed` are written to the service log as the stream produces them, not after the invocation.

Completed in Knot 0.45.0: `ensure_pi_compaction_enabled` startup self-heal in `src/server.rs` (D1 — merge-write beside the plan-080 warning, which remains the fallback with a reason); RPC adapter overflow test parity with the JSON adapter (D3 — recovered-overflow success and terminal `ContextLimitReached` through the mock-CLI harness); the continuity invariant test (D4 — a compaction-bearing re-entry re-enters with `--session-id <captured id>`); live span emission (D5 — a `CompactionObservation` observer callback on `execute_with_config_and_observer` in both pi runners, the post-hoc `log_compactions` removed, failed/aborted ends now logged as `ContextCompactionFailed` instead of filtered out; the two superseded plan-079 tests rewritten, not deleted). `ContextCompacted` is unchanged in shape and still success-only. No rig-document migration (knot-update 0.45.0 entry).

Full details in [088-compaction-assurance/compaction-assurance-plan.md](088-compaction-assurance/compaction-assurance-plan.md).

### 87. Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget

**Status:** ✅ Complete (2026-09-10) — released in v0.44.0
**Created:** 2026-09-10
**Goal:** Extend plan 086's self-continuation (a knot resuming a bounded task chain after its context water-mark, delivered into its own existing input) to **event-source** knots (`strand-dir: event:<producer>:<EventId>`) — which v1 left out — by delivering the stamped continuation into the knot's existing event dispatch dir so its own watcher re-triggers it; and replace 086's absolute `batch-deadline-epoch` (which subtracts queue wait) with a **queue-wait-exempt batch execution budget** (`budget-secs`, the remaining seconds carried across hops and decremented only by execution time), so a lengthy event queued between a handoff and the continuation's dequeue never shrinks the continuation's budget.

Full details in [087-event-source-continuation-and-queue-budget/087-event-source-continuation-and-queue-budget-plan.md](087-event-source-continuation-and-queue-budget/087-event-source-continuation-and-queue-budget-plan.md).

### 86. Graceful Task Handoff — Checkpointed Continuation Chains for Task-Bearing Knots

**Status:** ✅ Complete (2026-09-08) — released in v0.43.0
**Created:** 2026-09-08
**Goal:** Let a knot marked `task-loop` work through a durable, **rig-owned checklist** across a chain of bounded sessions, **handing off gracefully** when its context water-mark is crossed and resuming from the checklist — instead of accumulating to an overflow (single-session) or re-deriving scope per task (per-event sessions). Two tiers degrade but always recover: the plan-084 water-mark `steer` re-pointed at a handoff that wraps up (full tie-off **and/or** a `TasksIncomplete` event) (`pi-rpc` only), and the existing overflow/timeout terminal handling, where the re-dispatched idempotent knot resumes from the checklist (all adapters). Knot stays **task-blind**: it neither owns nor parses the checklist (format, location, and authorship — e.g. a planner knot writing a phase checklist the phase runner works from — are the rig designer's); the agent-emitted `TasksIncomplete` is the only task-specific seam, and its body is a **pointer block** to durable state (the checklist + committed files), never a re-statement of context. A **continuation never resets the timer** — it derives its `profile_timeout` from the batch deadline stamped on its event, so the whole chain obeys the original clock ("fresh context, never a fresh budget"); a `max_continuations` cap and the deadline bound the chain. The earlier draft's `tasks-per-session` count gate is dropped (a count heuristic, superseded by the water-mark where steering exists and by overflow recovery elsewhere — `pi-rpc` is the recommended adapter for task-bearing knots).

Full details in [086-graceful-task-handoff/graceful-task-handoff-plan.md](086-graceful-task-handoff/graceful-task-handoff-plan.md).

### 85. Event-Parse Log Flags and Anchored Rig `.gitignore` Entry

**Status:** ✅ Complete (2026-09-07) — released in v0.41.1
**Created:** 2026-09-07
**Goal:** Show the `occurred` boolean next to each event id on the `event parse …` console line (so active events and `occurred: false` acknowledgements are distinguishable), and anchor the auto-appended rig `.gitignore` entry with a leading slash (`/{basename}/`) — force-migrating any existing bare `{basename}/` line in place — so it ignores only the top-level rig dir and no longer over-matches nested directories such as `tie-offs/rig/`.

Completed in Knot 0.41.1: the event-parse diagnostic renders `PlanCreated=true, SpecReviewed=false` (pure stderr change — the `occurred` dispatch filter in `process_strand.rs` is untouched); `ensure_rig_repo()` in `git_versioner.rs` now appends `/{basename}/` and force-migrates a pre-existing bare line in place (anchored line, or marker-without-entry, short-circuit to no-ops; the tracked-by-parent guard is unchanged, and its logged untrack command now reads `git rm -r --cached /rig/`). Tests: the three existing entry tests assert the anchored form; new `ensure_rig_repo_migrates_unanchored_entry` (old shape converges with marker + user lines preserved, no duplicate, second run a no-op) and `ensure_rig_repo_leaves_anchored_entry` (byte-identical). Docs: release notes v0.41.1, knot-update 0.41.1 changelog, knot-init 4.10.0. No rig-document migration.

Full details in [085-event-log-flags-and-anchored-gitignore/085-event-log-flags-and-anchored-gitignore-plan.md](085-event-log-flags-and-anchored-gitignore/085-event-log-flags-and-anchored-gitignore-plan.md).

### 84. Graceful Completion — Steer the Session to Wrap Up Before Context Runs Out

**Status:** ✅ Complete (2026-09-07) — released in v0.42.0
**Created:** 2026-09-07
**Goal:** Add an opt-in `pi-rpc` agent runner (pi's JSONL command protocol over stdin, side-by-side with `pi-json`) that watches the session's live context usage and, when it crosses a per-alias `ctx-wrap-up-limit` (tokens, set in `rig/models.yml`, below the model's effective compaction point), **steers** the running agent — via pi's RPC `steer` command, delivered at the next turn boundary — to stop starting new work, commit all complete work, update its progress, note what is incomplete and where it left off, and produce its final tie-off; one steer per run, recorded as a `ContextWrapUpSteered` loom-log entry, so context exhaustion ends in a clean handoff instead of an uncommitted working tree. `pi-json` removal is the explicitly deferred next change.

Completed in Knot 0.42.0: the `pi-rpc` adapter (`src/adapters/pi_rpc.rs`) speaks pi's `--mode rpc` JSONL protocol over stdin/stdout — a reader thread forwards each stdout line to a driver thread, which sends the initial `prompt` and then samples `get_session_stats` on every `turn_end` (usage always captured for observability, parity with `pi-json`); when a `ctx-wrap-up-limit` is set and the sampled context tokens cross it, the driver sends one `steer` (the wrap-up prompt) and records the result. The config key `ctx-wrap-up-limit` (tokens, per model alias in `rig/models.yml`) resolves through `AgentConfig`/`ModelRef` (a zero filters to `None`, disabling steering), and `agent-adapter: pi-rpc` is the new rig-level value in `rig/.workspace-agent-config.yaml` (`AgentAdapter::PiRpc`). A one-shot `ContextWrapUpSteered` loom event (rendered in the service log, round-tripped in serde) is emitted from the session-resume Ok path via `WrapUpRecord` in `AgentInvocationMetadata`. `pi-json` is unchanged and remains the default; its removal is the deferred next change. Test strategy: the unit tests drive the real adapter end-to-end against a mock `pi` binary (`with_cli_path`), covering success (response/session-id/usage captured), steer-fires-once, no-steer-below-limit, and steer-despite-disabled-compaction.

### 83. Consolidated Service Log + Change-Driven State Writes

**Status:** ✅ Complete (2026-09-07) — released in v0.41.0
**Created:** 2026-09-07
**Goal:** Retire the per-run `.loom-log`/`.rig-log` JSONL files in favour of one consolidated, durable service log (single-line `[KNOT][EVENT]` / `[KNOT][STATE]` records on stderr, appended to `tie-offs/<rig>/knot-service.log` by the `knot-start` skill), keep run activity in-memory per process (nothing cleared at startup — the 072 startup-clear is retired), and make `state.json` writes change-driven (a no-op tick writes nothing, so an unchanged mtime means the rig is idle; `updated_at` records the last actual change).

Completed in Knot 0.41.0: `[EVENT]` lines render every `LoomEvent` (18 variants) and `RigLogEvent` variant as one line, field names mirroring the domain structs, `None` fields omitted (pinned per variant in unit tests over the pure renderers in `src/adapters/service_log.rs`); run activity lives in `src/application/activity.rs` (`RunActivity` + the `InMemoryLoomLog`/`InMemoryRigLog` port impls that emit on append and answer the existing activity/knot-status queries); the state writer now diffs the freshly derived state against the last written state (ignoring `updated_at`) and skips no-op writes — each real write logs `[STATE]` delta lines (initial snapshot / `change knot …: status idle→completed` / `change queue± …` / …), rendered from `diff_state` in `src/domain/state_change.rs`; the `StartupOptions`/startup-clear machinery of 072 is removed (`run_startup` is 2-arg); the deprecated file adapters (`FileSystemLoomLog`, `FileSystemRigLog`) are deleted and legacy `.loom-log`/`.rig-log` files are inert. Test strategy: binary-level subprocess tests (`tests/consolidated_log.rs`) pin the live `[EVENT]`/`[STATE]` stderr lines and the no-log-files invariant; in-process tests assert durable surfaces (`state.json`, tie-off sections, event-queue files). All skills and docs updated; `knot-update` carries the 0.41.0 entry (no document migration required).

Full details in [083-consolidated-service-log/consolidated-service-log-plan.md](083-consolidated-service-log/consolidated-service-log-plan.md).

### 82. System Event Subscriptions — Strand Off Any Knot Event, with Wildcard Producers

**Status:** ✅ Complete (2026-09-07) — released in v0.41.0
**Created:** 2026-09-07
**Goal:** Make every system event the rig writes (`LoomEvent`/`RigLogEvent` variant names) subscribable through `strand-dir: event:<producer>:<VariantId>` — with `*` accepted as a wildcard producer (any knot, loom, or the rig) — by dispatching system-produced events through the existing dispatch machinery, so recovery, reporting, and janitor knots can react to knot outcomes and exceptions (failure, timeout, inactivity, context overflow, empty response); `EventsDispatched` is the single non-dispatchable exception (it is the dispatch record itself).

Completed in Knot 0.41.0: `EventSubscription` gains `Wildcard` + `RigLevel` producer positions with new `resolve_loom_event` / `resolve_rig_event` resolvers; a new `SystemEventEmitter` (`EventScope` Knot/Loom/Rig) dispatches all 25 system events through the shared `dispatch_grouped` machinery across run outcome, retry/session, and loom/knot/rig lifecycle, with self-exclusion (a knot is never re-triggered by its own terminal event); acceptance via the mock-CLI harness (failure/success/self-exclusion/timeout). Docs updated: knot-create (System Events catalog), knot-design (loop discipline), knot-update (0.41.0 changelog), concepts.md.

Full details in [082-system-event-subscriptions/system-event-subscriptions-plan.md](082-system-event-subscriptions/system-event-subscriptions-plan.md).

### 81. Inactivity Timeout — Kill Blocked Sessions, Restart with a Blocking-Call Note

**Status:** ✅ Complete (2026-09-03) — released in v0.40.0
**Created:** 2026-09-02
**Goal:** Add a rig-global inactivity watchdog (`inactivity-timeout-seconds`, default 300, `0` disables) that detects a silent pi session at byte level (no thinking, no response, no streamed tool output), kills it at the window, and restarts it with a cause-specific blocking-call note — so a hung command stops the session within the window and the agent is told how to keep the session alive (background + poll, or stream output), instead of the rig sitting silent until the total budget expires; make `pi-json` the default adapter for **fresh** rigs (existing rigs keep their explicit setting).

Completed in Knot 0.40.0: reader + watchdog threads in both pi adapters (inactivity checked first; the total-budget timer is unchanged — the two timers are orthogonal: silence bounded by inactivity, work by the budget); `PortError::AgentInactivity` (resumable — the one error that may retry without a session ID, since knots are idempotent); blocked-call identification from the stream under `pi-json` (last unmatched `tool_execution_start`, e.g. `bash("npm run build")`); `AgentInactivity` loom-log event per stall (attempt, silent/window seconds, session ID, blocked call); restart with the `INACTIVITY_RESTART_NOTE` prompt note (replacing the generic final-response request for that attempt); exhaustion → cause-accurate terminal `AgentInactivity` → `TimeoutSkipped` outcome (no tie-off write) + `TimeoutExceeded` rig-log entry; `knot-init` 4.8.0 seeds `agent-adapter: pi-json` for fresh rigs. Empirically verified end-to-end against a real `pi` binary (kill with named blocked call → note-driven restart → completed tie-off). No rig-document migration (additive config key).

Full details in [081-inactivity-timeout/inactivity-timeout-plan.md](081-inactivity-timeout/inactivity-timeout-plan.md).

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
