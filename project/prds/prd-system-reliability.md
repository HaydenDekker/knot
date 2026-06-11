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
- [ ] Users can replay one or more previously processed events through a knot, using either the original agent profile or a modified one
- [ ] Users can roll back their tie-off output to an earlier point in time, discarding later processing events
- [ ] Users are alerted when a knot-to-knot (k2k) recursive feedback loop is detected, whether self-recursive or cross-knot
- [ ] Users can set a maximum iteration limit for k2k feedback loops so that refinement cycles terminate automatically if agents do not converge
- [ ] Users can set a per-agent-profile session timeout so that a hung or excessively slow agent session is terminated automatically

## Non-Goals

- Integration with provider billing APIs for exact cost tracking — estimates based on token counts are sufficient
- Multi-provider failover or load balancing — one provider per knot is the model
- Real-time cost alerting or notification webhooks — HTTP visibility is the interface
- Version control integration (git) for rollback — rollback operates on tie-off files and loom-log state
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

As a user, I want to replay one or more previously processed events through a knot, so that I can reprocess strands with a corrected agent profile or prompt template.

**Scenarios:**

1. Given a strand was previously processed and produced an unsatisfactory tie-off, when I request a replay of that event, then Knot reprocesses the strand using the current knot configuration and appends a new section to the tie-off
2. Given I have modified a knot's agent profile (e.g. changed the model or prompt), when I replay a past event, then the replay uses the updated configuration — not the original one
3. Given multiple strands were processed with a bad configuration, when I replay a batch of events, then each strand is reprocessed in sequence and the tie-offs are updated
4. Given I replay an event, when I check the loom-log, then the replay event is recorded with a distinct event type (e.g. `Replayed`) so I can distinguish replays from original processing

### Story 6: Rollback

As a user, I want to roll back my tie-off output to an earlier point in time, so that I can undo a batch of processing that produced incorrect results.

**Scenarios:**

1. Given a loom has processed 10 strand events, when I roll back to the state after event 5, then tie-off files are restored to their content after the 5th event and events 6–10 are removed from the loom-log
2. Given I roll back a loom, when I check the HTTP interface, then the loom state reflects the rolled-back position and queued events (if any) are cleared
3. Given I have rolled back, when new strand events fire or I replay events, then processing resumes from the rolled-back state with fresh tie-off sections

### Story 7: Knot-to-Knot Feedback Loop Detection and Control

As a user, I want to be alerted when a recursive feedback loop forms between knots — either a knot feeding its own output back into its input, or two independent knots writing outputs that trigger each other — so that I can intervene before unbounded iteration burns through my budget. I also want to define how many k2k (knot-to-knot) iterations are allowed before Knot forces a stop, in case the agents never naturally converge.

A typical example: a plan knot and an architecture review knot iterate on each other — the plan is created, the arch knot reviews the plan against current architecture and adds adjustments, the plan knot triggers on the dependency change and updates the plan, the arch knot reviews again, and so on. Ideally this loop ends naturally when both agents are satisfied, but if not a `max_k2k_iterations` cap must kick in and force a stop.

**Scenarios:**

1. Given knot A watches a directory that knot B writes to, and knot B watches a directory that knot A writes to, when A processes a strand and its output triggers B which triggers A again, then Knot detects the feedback cycle and logs a `FeedbackLoopDetected` event in the loom-log with the chain of knots involved
2. Given I have set `max_k2k_iterations` to 5 on a loom, when a feedback loop between two knots exceeds 5 iterations, then Knot stops processing further events in the chain and records a `FeedbackLoopExceeded` event with the iteration count
3. Given I have not set `max_k2k_iterations`, when a feedback loop is detected, then Knot alerts via the loom-log and HTTP interface but continues processing (no forced stop — the agents may converge naturally)
4. Given an architecture refinement loop where a plan knot and an architecture review knot iterate on each other, when the agents naturally converge (no further output changes after a full cycle), then Knot marks the loop as resolved and normal processing resumes
5. Given a feedback loop has been exceeded, when I check the HTTP interface, then I see which knots are in the cycle, how many iterations occurred, and the option to raise the limit or break the cycle
6. Given a single knot's tie-off directory overlaps with its own strand directory, when Knot starts, then it detects the self-recursive configuration at registration time and reports it as an error or warning (depending on configuration)

### Story 8: Agent Session Timeout

As a user, I want to define an agent timeout value based on my agent profile so that I can allow a slower model sufficient time to complete while limiting damage from a severe failure (e.g. a hung session that never returns).

**Scenarios:**

1. Given I have set `timeout` to 300 seconds on an agent profile, when an agent session exceeds that duration, then Knot terminates the session and records a `TimeoutExceeded` error in the tie-off and knot-state
2. Given I have two agent profiles — one using a fast model (60s timeout) and one using a slow model (600s timeout) — when each knot processes a strand, then each session uses the timeout configured on its profile
3. Given I have not set a timeout on an agent profile, when an agent session runs, then Knot uses a sensible default (e.g. 300 seconds) to prevent indefinite hangs
4. Given an agent session times out, when I check the HTTP interface, then I can see the timed-out event with its duration and the option to replay

## Success Criteria

- [ ] A user can configure `max_concurrent` (per knot or loom) and burst events are queued rather than all firing simultaneously
- [ ] A user can configure `rate_limit` (requests per time window per provider) and excess requests are deferred automatically
- [ ] A user can configure `max_tokens` (per event, per knot, or per loom) and Knot stops processing when the cap is reached
- [ ] Usage statistics (request count, token usage) are queryable via the HTTP interface at loom, knot, and rig levels
- [ ] A user can replay individual or batch events via the HTTP interface, and replays use the current (potentially modified) knot configuration
- [ ] A user can roll back a loom to a previous event position via the HTTP interface, and tie-off files are restored accordingly
- [ ] Replay and rollback events are recorded in the loom-log with distinct event types for auditability
- [ ] Feedback loops (self-recursive and cross-knot) are detected and logged with a `FeedbackLoopDetected` event; exceeding `max_k2k_iterations` produces a `FeedbackLoopExceeded` event and stops processing
- [ ] A user can configure `timeout` (per agent profile) and hung sessions are terminated with a `TimeoutExceeded` error
- [ ] All new configuration fields are validatable in the knot file parser (domain layer) — invalid values reject at parse time

## Dependencies & Constraints

- **Technical dependency:** The loom-log (JSONL) and tie-off append format provide the structured history needed for replay and rollback. Replay reads from the loom-log to find past events; rollback truncates the loom-log and reconstructs tie-off content from its `---`-separated sections.
- **Technical constraint:** Token usage tracking requires the agent CLI (`pi`) to report token counts in its output. If `pi` does not currently expose token metrics, Knot must parse or request them — this is a prerequisite.
- **Technical constraint:** Rate limiting and concurrency control require a request queue and scheduler in the application layer, replacing or wrapping the current immediate-fire processing pipeline.
- **External dependency:** Cost estimation is based on token counts and known per-token pricing for the configured provider/model. Exact billing integration with provider APIs is out of scope.
- **Configuration constraint:** New limits (`max_concurrent`, `rate_limit`, `max_tokens`, `max_k2k_iterations`) are optional — Knot operates without them if not configured (backwards compatible).
- **Technical constraint:** Feedback loop detection requires Knot to track the event propagation graph — which knot's output wrote to which other knot's input directory — and detect cycles in that graph at runtime. Self-recursive loops (strand_dir overlaps tie_off_dir) can be detected statically at knot registration time.
- **Technical constraint:** Agent session timeout requires the subprocess runner to track session start time and be capable of killing the child process on timeout. The current `SubprocessAgentRunner` waits for completion with no interrupt path.

## Implementation Status: 🔵 Open
