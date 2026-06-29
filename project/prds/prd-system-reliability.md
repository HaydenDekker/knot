# PRD: System Reliability — Messaging Control, Replay and Rollback

## Problem

When Knot runs agent sessions against an LLM provider, the user has no control over **how many** requests are sent, **how expensive** they are, or what happens when things go wrong. Knot fires every strand event immediately with no rate limiting, no concurrency cap, and no visibility into token usage or cost. If a bulk file drop triggers dozens of events at once, the provider is overwhelmed, the user is charged unexpectedly, and there is no way to pause or recover.

Additionally, when an agent produces an unsatisfactory tie-off — perhaps because the prompt template was wrong, the model was wrong, or the strand content was noisy — the user has no way to **reprocess** that event with a corrected configuration. The only option is to manually re-trigger the strand (e.g. touch the file), hoping the same outcome doesn't repeat. Similarly, if a batch of processing produces incorrect tie-offs across many strands, there is no way to **roll back** the project to a point before the damage was done.

Knot needs **operational safety controls** so the user can protect their provider budget, recover from bad runs, reprocess events with corrected configurations, and prevent runaway feedback loops — all without manual file manipulation or data loss.

## Goals

- [ ] Users can set per-knot or per-loom limits on how many agent sessions run concurrently, preventing provider overload from burst events
- [ ] Users can set rate limits (max requests per time window) per provider, with automatic queuing and backoff
- [ ] Users can set budget or token caps (per knot, per loom, or global) and Knot stops processing when the cap is reached
- [ ] Users can see current token usage and cost estimates via the HTTP interface, so they can make informed decisions about processing
- [x] A rig-level event log (`rig-log`) records serious operational events (timeouts, queue idle) so the user (or an external agent) can monitor and react — *Plan 28*
- [x] Failed strands are reported via the rig-log; the user reprocesses by touching the strand file, triggering the normal file-watcher pipeline (no programmatic replay in the app) — *Plan 28*
- [x] Users can set a per-agent-profile session timeout so that a hung or excessively slow agent session is terminated automatically — *Plan 28*
- [x] Each successful knot run creates a git commit with structured message and tie-off body; opt-out per-knot via `git-versioned: false`; gracefully skips if not a git repo — *Plan 27*
- [ ] When an agent invocation fails mid-stream, Knot resumes the Pi session from where it left off (up to a retry limit) before falling back to a fresh session — so partial work is not lost
- [ ] Users can roll back their tie-off output to an earlier point in time, discarding later processing events
- [ ] Users are alerted when a knot-to-knot (k2k) recursive feedback loop is detected, whether self-recursive or cross-knot
- [ ] Users can set a maximum iteration limit for k2k feedback loops so that refinement cycles terminate automatically if agents do not converge


## Non-Goals

- Integration with provider billing APIs for exact cost tracking — estimates based on token counts are sufficient
- Multi-provider failover or load balancing — one provider per knot is the model
- Real-time cost alerting or notification webhooks — HTTP visibility is the interface
- Git versioning for rollback — the rollback feature itself operates on tie-off files and loom-log state. Git versioning is a separate safety mechanism (see Story 10)
- Support for rolling back strand (input) files — rollback affects tie-off (output) only
- Scheduling or cron-based processing — that is a separate feature
- Provider usage analytics dashboards — basic usage endpoints suffice

## User Stories

### Story 1: Concurrency Control

As a user, I want to limit how many knots process strands at the same time, so that a burst of file events does not overwhelm my provider with too many concurrent requests.

**Scenarios:**

1. Given I have set `max_concurrent` to 3 on a loom, when 10 strand events fire at once, then only 3 agent sessions run in parallel and the remaining 7 are queued
2. Given I have set `max_concurrent` to 1 on a knot, when multiple strand events fire for that knot, then each event is processed sequentially
3. Given I have not set any concurrency limit, when strand events fire, then Knot processes them at its default concurrency (unbounded or a reasonable default)

### Story 2: Rate Limiting

As a user, I want to set a maximum number of requests per minute for my provider, so that I stay within the provider's rate limits and avoid 429 errors.

**Scenarios:**

1. Given I have set `rate_limit` to 30 requests per minute on a knot, when the 31st event arrives within the same minute, then it is deferred until the window resets
2. Given a provider returns a 429 response, when Knot detects the rate limit error, then it backs off automatically and retries the request after the recommended delay
3. Given rate limiting is active and events are being deferred, when I check the HTTP interface, then I can see how many events are queued and their estimated start time

### Story 3: Budget and Token Caps

As a user, I want to set a maximum budget or token limit for my processing, so that I am not unexpectedly charged more than I intend.

**Scenarios:**

1. Given I have set a `max_tokens` cap of 100,000 per loom, when processing reaches that limit, then Knot stops accepting new events for that loom and reports the budget cap in the HTTP interface
2. Given I have set a `max_tokens_per_event` cap, when a single strand processing exceeds the per-event limit, then the agent session is terminated and the tie-off records a budget exceeded error
3. Given a budget cap has been reached, when I clear the cap (raise the limit or reset), then Knot resumes processing queued events

### Story 4: Usage Visibility

As a user, I want to see how many tokens and requests I have used, so that I can understand my spending and decide whether to continue processing.

**Scenarios:**

1. Given I have processed several strands, when I query the HTTP interface for usage, then I see request counts, token usage (input + output), and per-knot breakdowns
2. Given usage has exceeded 80% of my configured budget cap, when I check the HTTP interface, then I see a warning indicator for that loom or knot
3. Given I have multiple looms, when I query usage at the rig level, then I see aggregate totals across all looms

### Story 5: Event Replay

As a user, I want to reprocess strands after a failure or with a corrected configuration. Knot does **not** provide programmatic replay — instead, failures are logged and the user triggers reprocessing by touching the strand file, which fires through the normal file-watcher pipeline.

The user (or an external agent monitoring the rig-log) decides what to do with failures — retry immediately, switch to a backup profile, or defer. The app's role is to surface failures and provide the mechanism to retrigger.

**Scenarios:**

1. Given a strand processing failed (e.g. timeout), when I `touch` the strand file, then Knot detects the file modification event and reprocesses the strand through the normal file-watcher pipeline using the current knot configuration
2. Given I have modified a knot's agent profile before reprocessing (e.g. switched to a backup provider), when I `touch` the strand file, then the reprocessing uses the updated configuration
3. Given I reprocess a strand by touching the file, when I check the loom-log, then the new processing event is recorded as a normal `Modified` event — indistinguishable from any other strand modification

### Story 5a: Session Resume on Invocation Failure

As a user, when an agent invocation fails partway through (network error, provider timeout mid-stream, subprocess killed), I want Knot to automatically resume the Pi session from where it left off — up to a configurable retry limit — so that partial work is not lost and the provider does not re-process the full context from scratch.

This is distinct from Story 5 (Event Replay). Event replay is **user-triggered** (touch the file) and always starts a **fresh session**. Session resume is **automatic**, happens **before** the strand is marked as failed, and continues the **same Pi session** using `--session-id` so the provider sees a continuation of the conversation.

Knot captures the Pi session ID from the JSON-L output stream (first line: `{"type":"session","id":"..."}`). On retry, Knot re-invokes Pi with `--session-id <id>` — Pi appends the turn to the existing session file. If the session resumes successfully, the strand completes normally. If it fails again, Knot retries up to the configured limit, then marks the strand as failed and falls back to the normal failure path (rig-log entry, user can `touch` to restart fresh).

**Resumable failures** — Knot attempts session resume for these:
- Agent session timeout (Knot killed the subprocess via timeout)
- Provider returned an error response (non-429, non-auth errors visible in Pi's exit code or output)
- Subprocess killed unexpectedly (signal, crash) but session ID was captured

**Empty finish responses** — When the agent exits cleanly (exit-code 0) but produces no response text (e.g. the provider returned an empty finish), Knot logs a `KnotEmptyResponse` event to the loom-log and retries the session with a stronger guidance prompt ("you must provide a final response if finished or continue with the task") rather than the generic "please continue". The retry is counted against the overall timeout budget. If the agent eventually produces a response, the strand completes normally — the user sees the `KnotEmptyResponse` entries in the loom-log but the strand succeeds. If all retries also return empty, the strand fails with `TimeoutExceeded` in the rig-log. The tie-off file records only complete final responses.

**Fatal failures** — Knot does **not** attempt session resume for these (strand is marked as failed immediately):
- Configuration error (missing profile, invalid args, missing CLI binary)
- Provider returned a 429 rate limit — these are handled by rate limiting (Story 2), not resume
- Session ID was never captured (e.g. Pi failed to start, output was empty) — no session to resume
- Retry limit exhausted — strand is marked as failed, falls through to normal failure path

**Scenarios:**

1. Given an agent invocation fails partway through due to a provider network error, when the session ID was captured, then Knot automatically resumes the session using `--session-id` and the agent continues from where it left off
2. Given an agent session times out and the session ID was captured, when Knot retries with `--session-id`, then Pi resumes the conversation and the agent can complete the response
3. Given session resume succeeds on retry, when I check the loom-log, then the strand shows a single successful `KnotCompleted` entry — the retry is an implementation detail, not a separate event
4. Given session resume fails again, when Knot retries up to the configured `max_retries` limit, then after exhausting retries the strand is marked as failed and a `TimeoutExceeded` / `KnotFailed` entry is written to the loom-log and rig-log
5. Given the retry limit is exhausted, when I `touch` the strand file to reprocess, then a fresh Pi session is started (the old session is abandoned) — this falls back to the normal Event Replay path (Story 5)
6. Given a configuration error (e.g. missing CLI binary), when the invocation fails, then Knot does **not** attempt session resume — the strand is marked as failed immediately
7. Given I have not configured `max_retries`, when an invocation fails, then Knot uses a default (e.g. 1 retry — one resume attempt before giving up)
8. Given session resume is attempted, when I check the loom-log, then I see a `SessionResumed` entry indicating a retry with the original session ID, so I can trace the retry chain
9. Given an invocation fails and the session ID was never captured (Pi failed before outputting the session line), when Knot processes the failure, then it marks the strand as failed immediately — no session to resume
10. Given `max_retries` is set to 0 on a knot, when an invocation fails, then Knot does not attempt session resume — strand is marked as failed immediately (resume is disabled for that knot)
11. Given the agent exits cleanly but produces no response text (empty finish), when the session ID was captured, then Knot logs a `KnotEmptyResponse` event to the loom-log and retries with the guidance prompt "you must provide a final response if finished or continue with the task"; if the retry produces output the strand completes normally, and if all retries also return empty the strand is marked as failed
12. Given an empty finish retry succeeds, when I check the loom-log, then I see `KnotEmptyResponse` entries for the empty attempts and a `KnotCompleted` entry — the empty responses are recorded but the strand ultimately succeeded

### Story 6: Rig-Log Notification

As a user, I want a rig-level event log that records only serious operational events, so that I (or an external agent) can monitor the rig and react to failures without being flooded with noise. The rig-log is a single file at the rig root that the user watches externally — it is not an in-memory notification system.

The rig-log records a small set of high-signal events:
- **Agent session timeout** — a knot's agent session was killed because it exceeded its timeout
- **Queue idle** — the event pipeline has no pending events and all processing has completed

Events that do **not** appear in the rig-log:
- Knot completion — too noisy, user can check loom-log if needed
- Loom/knot registration errors — the user is actively changing configuration at that time and can check the loom-log directly
- Strand processing success — only failures and idle are worth surfacing
- Strand event issues (temp files, missing files, binary files) — these are logged to the loom-log and console. Known temp files (e.g. `sed -i` macOS temp files) are silently skipped with no log entry. Unknown missing files produce a `StrandSkipped` entry in the loom-log plus a console warning. Binary files produce a `StrandIgnored` entry. These are strand-level operational details, not rig-level alerts.

The rig-log enables the user to build their own replay logic: monitor the log, see a timeout, decide whether to touch the file or switch profiles, and act accordingly. The user may be a human reading the log, or an LLM agent watching the log and making decisions.

**Scenarios:**

1. Given an agent session times out, when I watch the rig-log file, then I see a `TimeoutExceeded` entry with loom, knot, strand path, and duration
2. Given a burst of strand events has all completed, when I watch the rig-log file, then I see a `QueueIdle` entry indicating no pending events
3. Given a strand processes successfully, when I watch the rig-log file, then no entry is written — success is silent
4. Given a knot fails to register due to a config error, when I watch the rig-log file, then no entry is written — the error is in the loom-log for the user to check while they fix configuration
5. Given the rig-log receives entries, when I am an external agent (e.g. an LLM) watching the file, then I can read the entries and decide to `touch` strand files to reprocess or modify profile configuration
6. Given the rig-log is watched by multiple consumers, when Knot writes entries, then the log is appended atomically so no consumer sees partial entries
7. Given Knot starts with existing rig-log content from a previous session, when I watch the rig-log, then I see both old and new entries — the log is persistent and append-only

### Story 7: Rollback

As a user, I want to roll back my tie-off output to an earlier point in time, so that I can undo a batch of processing that produced incorrect results.

**Scenarios:**

1. Given a loom has processed 10 strand events, when I roll back to the state after event 5, then tie-off files are restored to their content after the 5th event and events 6–10 are removed from the loom-log
2. Given I roll back a loom, when I check the HTTP interface, then the loom state reflects the rolled-back position and queued events (if any) are cleared
3. Given I have rolled back, when new strand events fire or I replay events, then processing resumes from the rolled-back state with fresh tie-off sections

### Story 8: Knot-to-Knot Feedback Loop Detection and Control

As a user, I want to be alerted when a recursive feedback loop forms between knots — either a knot feeding its own output back into its input, or two independent knots writing outputs that trigger each other — so that I can intervene before unbounded iteration burns through my budget. I also want to define how many k2k (knot-to-knot) iterations are allowed before Knot forces a stop, in case the agents never naturally converge.

A typical example: a plan knot and an architecture review knot iterate on each other — the plan is created, the arch knot reviews the plan against current architecture and adds adjustments, the plan knot triggers on the dependency change and updates the plan, the arch knot reviews again, and so on. Ideally this loop ends naturally when both agents are satisfied, but if not a `max_k2k_iterations` cap must kick in and force a stop.

**Scenarios:**

1. Given knot A watches a directory that knot B writes to, and knot B watches a directory that knot A writes to, when A processes a strand and its output triggers B which triggers A again, then Knot detects the feedback cycle and logs a `FeedbackLoopDetected` event in the loom-log with the chain of knots involved
2. Given I have set `max_k2k_iterations` to 5 on a loom, when a feedback loop between two knots exceeds 5 iterations, then Knot stops processing further events in the chain and records a `FeedbackLoopExceeded` event with the iteration count
3. Given I have not set `max_k2k_iterations`, when a feedback loop is detected, then Knot alerts via the loom-log and HTTP interface but continues processing (no forced stop — the agents may converge naturally)
4. Given an architecture refinement loop where a plan knot and an architecture review knot iterate on each other, when the agents naturally converge (no further output changes after a full cycle), then Knot marks the loop as resolved and normal processing resumes
5. Given a feedback loop has been exceeded, when I check the HTTP interface, then I see which knots are in the cycle, how many iterations occurred, and the option to raise the limit or break the cycle
6. Given a single knot's tie-off directory overlaps with its own strand directory, when Knot starts, then it detects the self-recursive configuration at registration time and reports it as an error or warning (depending on configuration)

### Story 9: Agent Session Timeout

As a user, I want to define an agent timeout value based on my agent profile so that I can allow a slower model sufficient time to complete while limiting damage from a severe failure (e.g. a hung session that never returns).

**Scenarios:**

1. Given I have set `timeout` to 300 seconds on an agent profile, when an agent session exceeds that duration, then Knot terminates the session, logs `KnotFailed` with a timeout error in the loom-log, writes a `TimeoutExceeded` entry to the rig-log, and does **not** write any content to the tie-off file — the previous tie-off (if any) is preserved unchanged
2. Given I have two agent profiles — one using a fast model (60s timeout) and one using a slow model (600s timeout) — when each knot processes a strand, then each session uses the timeout configured on its profile
3. Given I have not set a timeout on an agent profile, when an agent session runs, then Knot uses a sensible default (e.g. 300 seconds) to prevent indefinite hangs
4. Given an agent session times out, when I watch the rig-log file, then I see a `TimeoutExceeded` entry with loom, knot, strand path, and duration — this is how I discover the failure
5. Given an agent session times out and I want to retry, when I `touch` the strand file, then Knot reprocesses the strand through the normal file-watcher pipeline and writes a fresh tie-off if successful
6. Given an agent session times out, when I inspect the tie-off file on disk, then it contains only the previous successful content (if any) — no error message was appended
7. Given an agent session times out, when I inspect the loom-log, then it contains `KnotProcessing`, `KnotFailed` (with timeout error), and `StrandProcessed` (with error) entries that fully describe what happened

### Story 10: Git Versioning

As a user, I want each knot run to produce a git commit in my project, so that I have a permanent, auditable history of agent work and can revert bad changes using standard git tools.

**Scenarios:**

1. Given my project is a git repository and a knot processes a strand, when processing completes, then a git commit is created in the project root containing all changes (tie-off + any working tree modifications)
2. Given a commit was created, when I inspect `git log`, then the commit message includes the loom, knot, strand, and event type (e.g. `knot: review-knot — processed strands/goals.md (Created)`)
3. Given a commit was created, when I inspect the commit body, then it contains the tie-off output (the current response, not the full appended trail)
4. Given I have set `git-versioned: false` on a knot's frontmatter, when that knot processes a strand, then no commit is created (other knots still commit normally)
5. Given my project is not a git repository (or git is unavailable), when a knot processes a strand, then Knot skips versioning gracefully — processing succeeds with no error
6. Given multiple knots process strands in series, when all processing completes, then each knot run produces its own separate commit (no batching)

## Success Criteria

- [ ] A user can configure `max_concurrent` (per knot or loom) and burst events are queued rather than all firing simultaneously
- [ ] A user can configure `rate_limit` (requests per time window per provider) and excess requests are deferred automatically
- [ ] A user can configure `max_tokens` (per event, per knot, or per loom) and Knot stops processing when the cap is reached
- [ ] Usage statistics (request count, token usage) are queryable via the HTTP interface at loom, knot, and rig levels
- [x] A rig-log file exists at the rig root and receives `TimeoutExceeded` entries when agent sessions time out, and `QueueIdle` entries when the event pipeline has no pending events — *Plan 28*
- [x] The rig-log is append-only and persistent — entries survive server restarts — *Plan 28*
- [x] Successful processing, loom/knot registration, and config errors do **not** appear in the rig-log — *Plan 28*
- [ ] On invocation failure (timeout, network error), Knot captures the Pi session ID and attempts to resume the session via `--session-id` up to `max_retries` times before marking the strand as failed
- [ ] Successful session resume is transparent — the strand shows a single `KnotCompleted` entry; failed retries produce `SessionResumed` entries in the loom-log for traceability
- [ ] `max_retries` is configurable per knot (default: 1); setting it to 0 disables session resume entirely
- [x] A user can reprocess a failed strand by touching the strand file, and the reprocessing fires through the normal file-watcher pipeline — *Plan 28 (file-watcher pipeline exists, rig-log surfaces failures)*
- [x] On timeout, the tie-off file is preserved unchanged — no error content is appended — *Plan 28*
- [x] A user can configure `timeout` (per agent profile) and hung sessions are terminated with a `TimeoutExceeded` error — *Plan 28*
- [ ] A user can roll back a loom to a previous event position via the HTTP interface, and tie-off files are restored accordingly
- [ ] Replay and rollback events are recorded in the loom-log with distinct event types for auditability
- [ ] Feedback loops (self-recursive and cross-knot) are detected and logged with a `FeedbackLoopDetected` event; exceeding `max_k2k_iterations` produces a `FeedbackLoopExceeded` event and stops processing
- [x] A user can set `git-versioned: false` on a knot to opt out of versioning, and no commit is created for that knot
- [x] Each successful knot run creates a git commit in the project root (if it is a git repository)
- [x] Commit message includes loom, knot, strand, and event type (e.g. `knot: review — processed strands/goals.md (Created)`)
- [x] Commit body includes the tie-off output (current response only, not full history)
- [x] Multiple knot runs produce separate commits (one per strand event)
- [x] Non-git repositories or unavailable git are handled gracefully — processing succeeds without error
- [ ] All new configuration fields are validatable in the knot file parser (domain layer) — invalid values reject at parse time

## Dependencies & Constraints

- **Technical dependency:** The loom-log (JSONL) and tie-off append format provide the structured history needed for replay and rollback. Replay reads from the loom-log to find past events; rollback truncates the loom-log and reconstructs tie-off content from its `---`-separated sections.
- **Technical constraint:** Token usage tracking requires the agent CLI (`pi`) to report token counts in its output. If `pi` does not currently expose token metrics, Knot must parse or request them — this is a prerequisite.
- **Technical constraint:** Rate limiting and concurrency control require a request queue and scheduler in the application layer, replacing or wrapping the current immediate-fire processing pipeline.
- **External dependency:** Cost estimation is based on token counts and known per-token pricing for the configured provider/model. Exact billing integration with provider APIs is out of scope.
- **Configuration constraint:** New limits (`max_concurrent`, `rate_limit`, `max_tokens`, `max_k2k_iterations`) are optional — Knot operates without them if not configured (backwards compatible).
- **Technical constraint:** Feedback loop detection requires Knot to track the event propagation graph — which knot's output wrote to which other knot's input directory — and detect cycles in that graph at runtime. Self-recursive loops (strand_dir overlaps tie_off_dir) can be detected statically at knot registration time.
- **Technical constraint:** Agent session timeout requires the subprocess runner to track session start time and be capable of killing the child process on timeout. The current `SubprocessAgentRunner` waits for completion with no interrupt path.
- **Technical constraint:** Session resume requires the Pi session ID to be captured from the JSON-L output stream before the session completes. The session ID appears in the first JSON line (`{"type":"session","id":"..."}`), so it must be parsed from stdout early — before the failure occurs. This requires Knot to use `--mode json` and parse the JSON-L stream (see Demand Control PRD, Pi Integration section).
- **Design decision:** Session resume is limited to failures where the Pi process started and output began (session ID captured). If Pi fails to start (missing binary, config error), there is no session to resume — the strand fails immediately. This avoids Knot retrying against a fundamentally broken configuration.
- **Design decision:** On timeout, Knot writes error details to the loom-log and rig-log — **not** to the tie-off file. The tie-off is the agent's output and should contain only agent-produced content. Operational errors belong in logs where the user can observe and react. This keeps the tie-off clean for downstream consumers and allows the user to retry by simply touching the strand file.
- **Design decision:** Knot does **not** provide programmatic replay. The app's role is to surface failures (rig-log, loom-log) and provide the reprocessing mechanism (file-watcher pipeline). The user — human or external agent — monitors the rig-log and decides how to react: touch files to retry, switch profiles, or defer. This keeps the app focused on event processing and pushes replay policy to the user.

**Design decision:** Session resume (Story 5a) is an automatic best-effort recovery, not programmatic replay. When a strand invocation fails mid-stream, Knot attempts to resume the same Pi session (`--session-id`) up to `max_retries` times before giving up. This preserves partial work and avoids re-sending the full context. Once retries are exhausted, the strand is marked as failed and the user can `touch` the file for a fresh session — the normal Event Replay path (Story 5). Resume is transparent on success (single `KnotCompleted` entry); on failure, `SessionResumed` entries are logged for traceability.
- **Design decision:** Empty finish responses (agent exits cleanly with no output) are treated as resumable failures. A `KnotEmptyResponse` event is logged to the loom-log per-attempt, and the retry injects the guidance prompt "you must provide a final response if finished or continue with the task" — distinct from the generic "please continue" used for timeout/crash retries. On successful retry the strand completes normally; the user sees the `KnotEmptyResponse` entries in the loom-log but the strand succeeds. The tie-off records only complete final responses.
- **Design decision:** The rig-log records only high-signal events (timeout, queue idle) and excludes noisy events (completion, registration, config errors). Completion is too frequent to be useful. Config errors occur when the user is actively changing the system and can check the loom-log directly. The rig-log is the user's window into "something needs attention" without requiring polling or HTTP calls.
- **Design decision:** Strand event issues (temp files from `sed -i`, missing files, binary files) are logged to the loom-log and console — **not** the rig-log. Known temp files are silently skipped with no log entry. Unknown missing files produce a `StrandSkipped` loom-log entry plus console warning. Binary files produce a `StrandIgnored` loom-log entry. These are strand-level operational details, not rig-level alerts. The user checks the loom-log if concerned about repeated skips.
- **Technical constraint:** Git versioning requires the project root (parent of `rig/`) to be a git repository. The adapter must detect this and skip gracefully if it is not. The `git2` crate or a subprocess call to `git` can be used — subprocess avoids adding a C library dependency.
- **Technical constraint:** Git versioning runs after tie-off write in `ProcessStrand`. It must not fail the overall processing pipeline — if git commit fails, a warning is logged but the strand is still marked as completed.
- **Dependency:** The `base_dir` (rig directory) is known at composition root and can be used to derive the project root (parent of rig). This path is passed to the git adapter at construction time.

## Implementation Status: 🟡 Partial

**Completed stories:**
- Story 6 (Rig-Log Notification) — *Plan 28*
- Story 7 (Event Replay) — *Plan 28*
- Story 9 (Agent Session Timeout) — *Plan 28*
- Story 10 (Git Versioning) — *Plan 27*

**Remaining:** Stories 1–5 (Concurrency, Rate Limiting, Budget, Usage Visibility), Story 8 (Feedback Loop Detection), Story 7 (Rollback)
