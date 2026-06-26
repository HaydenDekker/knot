# PRD: Demand Control — Concurrency, Throughput and Service Tuning

## Problem

Knot processes strand events sequentially — one agent invocation at a time. When a user's LLM provider can handle higher throughput (faster models, higher rate limits, larger budgets), Knot becomes the bottleneck. The user has no way to increase parallelism to match their provider's capacity, and no visibility into how fast their agent invocations are completing.

Conversely, when a user's provider is slow or rate-limited, Knot fires events immediately with no awareness of whether the provider can absorb the demand. The user cannot tune Knot's outgoing demand to match their service's actual capacity.

Currently the user has **one knob** (serial processing) for **all situations** — whether they have a slow local model or a fast cloud API. They need:
1. A way to **increase parallelism** when their service can handle it
2. **Visibility** into invocation speeds so they can tune their configuration
3. A **global configuration** surface to set these values (none exists today)

## Goals

- [ ] Users can configure a **maximum parallel agent invocations** setting so that multiple strand events are processed concurrently rather than serially
- [ ] Users can see **recent invocation performance** (e.g. last 10 completions with duration) so they can understand their current throughput and decide whether to increase or decrease parallelism
- [ ] Invocation performance data is exposed via the HTTP interface for programmatic consumption
- [ ] A **global configuration file** exists at the rig level for settings that apply across all looms and knots (currently no global config surface exists)
- [ ] Users can see **token usage per invocation** (input, output, total tokens) to understand cost and throughput characteristics of their provider

## Non-Goals

- Automatic throughput control (Knot does not auto-adjust parallelism based on measured throughput — the user decides)
- Real-time streaming of generation speed (TPS) during a single invocation
- Provider billing integration or exact cost tracking
- Multi-provider load balancing or failover
- Per-provider rate limit enforcement (provider returns 429s; Knot does not pre-emptively throttle)
- Minimum throughput enforcement as a hard limit (if a provider is too slow, Knot reports it but does not stop processing)

## User Stories

### Story 1: Max Parallel Invocations

As a user with a fast LLM provider, I want to configure how many agent sessions can run at the same time, so that my provider's capacity is fully utilised when many strands need processing.

**Scenarios:**

1. Given I have set `max_parallel` to 4, when 10 strand events fire at once, then up to 4 agent sessions run concurrently and the remaining 6 wait in a queue
2. Given I have not set `max_parallel`, when strand events fire, then Knot processes them serially (one at a time) — backwards compatible default
3. Given I have set `max_parallel` to 1, when strand events fire, then each event is processed sequentially — explicit serial mode
4. Given I have set `max_parallel` to 4 and 3 sessions are running, when a 4th strand event arrives, then it starts immediately; when a 5th arrives, it waits until a slot frees

### Story 2: Invocation Performance Visibility

As a user, I want to see how fast my recent agent invocations completed, so that I can decide whether to increase or decrease my parallelism setting.

**Scenarios:**

1. Given I have processed several strands, when I query the HTTP interface, then I see the last 10 invocations with their duration (start time, end time, wall-clock seconds), strand path, knot name, and status
2. Given some invocations failed (timeout, error), when I view recent invocations, then failed invocations are shown with their error and duration (time until failure)
3. Given I have multiple looms and knots, when I view recent invocations, then I can see all invocations across the rig or filter by loom or knot
4. Given I am monitoring the rig from an external tool, when I query the invocations endpoint, then I receive structured JSON I can parse for duration statistics

### Story 3: Token Usage Per Invocation

As a user, I want to see token usage for each agent invocation, so that I can understand the cost and throughput characteristics of my provider.

**Scenarios:**

1. Given an agent invocation completed successfully, when I view its details, then I see input tokens, output tokens, cache read tokens, and total tokens
2. Given an invocation failed (timeout, error), when I view its details, then token fields are absent or zero — partial usage is best-effort
3. Given I have processed many strands, when I query usage at the rig level, then I see aggregate token totals across all invocations

### Story 4: Global Configuration

As a user, I want to set rig-level settings in a single configuration file, so that I don't need to configure the same values in every knot or loom.

**Scenarios:**

1. Given I have a global config file at the rig root, when I set `max_parallel: 4`, then all looms and knots respect this parallelism limit
2. Given I have set `max_parallel` globally, when I also set it per-loom, then the loom-level value overrides the global default for that loom only
3. Given no global config file exists, when Knot starts, then it uses sensible defaults (serial processing, no rate limits)

### Story 5: Throughput-Informed Tuning

As a user, I want to see recent invocation speeds alongside my current `max_parallel` setting, so that I can make informed decisions about whether to increase or decrease parallelism.

**Scenarios:**

1. Given I have `max_parallel: 2` and recent invocations average 8 seconds each, when I view the dashboard, then I see "2 concurrent × ~8s each ≈ ~4 strands/min throughput" as an estimated throughput figure
2. Given I increase `max_parallel` from 2 to 4 and observe invocations now take 15 seconds each (provider is under load), then I can see the trade-off and decide to reduce parallelism
3. Given I decrease `max_parallel` from 4 to 2 and observe invocations drop to 6 seconds each, then I can see the per-invocation improvement and calculate whether total throughput improved

## Success Criteria

- [ ] A user can set `max_parallel` (global or per-loom) and burst events are processed concurrently up to that limit
- [ ] The HTTP interface exposes a recent invocations endpoint showing last N completions with duration, status, and token usage
- [ ] Token usage (input, output, total) is captured per invocation and exposed via HTTP
- [ ] A global config file exists at the rig root with documented settings
- [ ] Default behaviour (no config) remains serial processing — backwards compatible
- [ ] Invocation performance data is also persisted in `rig/state.json` for file-based consumers

## Dependencies & Constraints

- **Technical constraint:** The current `ProcessStrand` loop in `server.rs` processes events sequentially (one `execute()` at a time). Parallel processing requires a semaphore or worker pool pattern.
- **Technical constraint:** Token usage is only available if Pi is invoked with `--mode json` and the JSON-L output is parsed. Currently Knot uses `--print` mode which outputs plain text to stdout. Switching to JSON mode would change the capture path — stdout would contain the JSON session stream, not the final text response.
- **Technical dependency:** Pi's `--mode json` output includes `usage` objects with `input`, `output`, `cacheRead`, `cacheWrite`, `totalTokens`, and `cost` fields. The `agent_end` event contains the final usage summary. This is parseable from stdout when `--mode json` is used.
- **Technical constraint:** Session IDs are generated by Pi at invocation time (visible in the first JSON line: `{"type":"session","id":"..."}`). Knot cannot predict the session ID beforehand — it is captured from output after the process starts.
- **Technical constraint:** Per-invocation TPS (tokens per second) during generation requires either parsing streaming `message_update` events in real-time (complex — Knot would need to buffer and parse JSON-L from the subprocess while also waiting for it to complete), or deriving it post-hoc from the final `usage` and `timestamp` fields.
- **Design decision:** "Min acceptable throughput" is NOT a hard control. Instead, Knot provides visibility into recent invocation durations and estimated throughput. The user adjusts `max_parallel` based on this information. This avoids Knot making autonomous decisions about user's provider capacity.
- **Design decision:** Global config lives in the rig directory. The existing `.workspace-agent-config.yaml` (which holds `cli_path` and `cli_args`) is the natural home for new rig-level settings like `max_parallel`. Alternatively, a new file name could be used if scope separation is desired.
- **Configuration constraint:** New settings are optional — Knot operates with defaults if they are not configured (backwards compatible).

## Implementation Status: 🔵 Open

## Exploration Notes

### Relationship to Existing PRDs

This PRD overlaps with **[System Reliability — Messaging Control, Replay and Rollback](prd-system-reliability.md)** which already defines:
- Story 1: Concurrency Control (`max_concurrent`) — conceptually similar to `max_parallel` here
- Story 2: Rate Limiting — requests per time window
- Story 4: Usage Visibility — token usage via HTTP

The System Reliability PRD frames these controls as **safety mechanisms** (protect the provider, control cost). This PRD frames them as **demand tuning** (maximise throughput, match provider capacity). Both perspectives are valid and the implementation would likely share infrastructure.

**Recommended approach:** The System Reliability PRD's Story 1 (concurrency) and Story 4 (usage visibility) could be implemented as part of this feature. The demand control perspective adds:
- Invocation performance visibility (recent N durations) — not in existing PRD
- Throughput estimation (concurrency × avg duration) — not in existing PRD
- Global configuration surface — not in existing PRD
- Token usage capture from Pi — prerequisite for both PRDs

### Pi Integration Options

Knot currently invokes Pi as a subprocess and captures stdout as plain text. To get token usage and session IDs, several options exist:

**Option A — Switch to `--mode json`:**
- Pi outputs JSON-L to stdout. Knot parses the stream.
- The `agent_end` event contains final usage data.
- The response text is extracted from the last `message_end` content.
- **Pros:** Full access to usage, session ID, timestamps, response ID. Single subprocess.
- **Cons:** Changes the capture path. Currently `AgentOutput.stdout` is the plain text response. Would need to extract text from the JSON stream. Adds parsing complexity.

**Option B — Dual invocation:**
- Run Pi normally for the response text.
- After completion, use the session ID to query Pi for usage separately.
- **Pros:** No change to existing capture path.
- **Cons:** Requires a second subprocess call. Session ID must be captured from the first invocation's output somehow. Adds latency.

**Option C — Stderr side-channel:**
- Pi outputs the response to stdout and usage metadata to stderr.
- **Pros:** Clean separation. No change to stdout parsing.
- **Cons:** Requires Pi to support this. Currently Pi does not separate usage to stderr.

**Option D — Post-hoc session query:**
- After subprocess completes, Knot reads the session file from `~/.pi/sessions/` (or wherever Pi stores them).
- **Pros:** No change to invocation. Session files contain full history.
- **Cons:** Depends on Pi's file format. May not be stable. Adds file I/O.

**Recommendation:** Option A (switch to `--mode json`) is the cleanest path. The subprocess already captures stdout — changing from text to JSON-L is a parsing change, not an architectural one. The `agent_end` event provides all needed data in one place.

### Throughput Derivation

Without real-time streaming, throughput can be derived post-hoc:

```
duration = end_timestamp - start_timestamp  (wall clock from Knot)
tokens = usage.output_tokens                (from Pi's agent_end event)
tps = tokens / duration                      (derived)
```

For the "last 10 invocations" view, Knot tracks a sliding window of recent completion records:
```json
{
  "invocation_id": "unique-id",
  "knot_name": "review-knot",
  "loom_id": "review-loom",
  "strand_path": "/path/to/file.md",
  "status": "completed",
  "started_at": "2026-06-26T10:00:00Z",
  "completed_at": "2026-06-26T10:00:12Z",
  "duration_seconds": 12.3,
  "tokens": {
    "input": 5000,
    "output": 200,
    "total": 5200
  },
  "estimated_tps": 16.3
}
```

### Configuration Surface

The existing `RigAgentConfig` (loaded from `.workspace-agent-config.yaml`) currently holds:
```yaml
cli_path: "pi"
cli_args: []
```

Extending it for demand control:
```yaml
cli_path: "pi"
cli_args: []
max_parallel: 4
```

This is the simplest path — one file, rig-level scope, extends existing config loading.

### Parallel Processing Architecture

The current pipeline is:

```
NotifyEventSource → mpsc::channel → DebounceEngine → InspectQueue → ProcessStrand loop (sequential)
```

For parallel processing, the `ProcessStrand` loop would need to become a **worker pool**:

```
NotifyEventSource → mpsc::channel → DebounceEngine → InspectQueue → [Semaphore-limited worker pool]
```

The semaphore limits concurrent `execute()` calls. Workers pull from the queue and process independently. The `QueueIdle` detection would need to account for in-flight work (idle = queue empty AND no workers active).

### Open Questions

1. **Should `max_parallel` be per-knot, per-loom, or rig-level?** — Start rig-level (global config), extend to per-loom later.
2. **How many recent invocations to retain?** — 10 is the initial request. Could be configurable.
3. **Is TPS meaningful across different models?** — TPS varies wildly by model, prompt size, provider load. Best shown as a per-invocation metric, not a target.
4. **Should Knot support `--mode json` and `--print` modes?** — For now, pick one. If `--mode json` becomes the standard, `--print` is not needed.
5. **What happens if `max_parallel` is set but provider rate-limits us?** — Knot does not currently handle 429s. That is a separate concern (System Reliability PRD, Story 2).
6. **Should the invocation history be persisted across restarts?** — Currently in-memory. For file-based consumers, it could be included in `rig/state.json`.
