# Troubleshooting

Common issues and how to resolve them.

## Knot Is Not Running

### Symptom

`tie-offs/<rig>/state.json` does not exist or is not being updated.

### Fix

Start Knot from your project directory:

```bash
cargo run
# or, if installed:
knot
```

To keep the output instead of losing it when the terminal closes, append
it to the service log and run in the background:

```bash
mkdir -p tie-offs/rig
nohup knot >> tie-offs/rig/knot-service.log 2>&1 &
echo $! > tie-offs/rig/knot-service.pid
```

Verify by watching the state file:

```bash
watch -n 2 'cat tie-offs/rig/state.json | python3 -m json.tool'
```

## The Service Keeps Dying

### Symptom

`tie-offs/<rig>/state.json` goes stale, then fresh, then stale again —
Knot starts and dies repeatedly.

### Why there is no other record

Run activity is **in-memory per process** (plan 083) — a crashed process
keeps no files of its own, so the appended service log (the
`[KNOT][EVENT]` / `[KNOT][STATE]` stderr lines) is the only record that
spans runs. If you did not append one, start doing so and reproduce the
failure:

```bash
nohup knot >> tie-offs/rig/knot-service.log 2>&1 &
echo $! > tie-offs/rig/knot-service.pid
grep -nE 'WARNING|Error|panic' tie-offs/rig/knot-service.log | tail -20
```

Stop a backgrounded Knot with `kill -INT $(cat tie-offs/rig/knot-service.pid)`.
Knot handles SIGINT only, so a plain `kill` (SIGTERM) ends it without
draining the queue — the unfinished event stays in `tie-offs/<rig>/events/`
and is re-queued on the next start.

Agents: use the `knot-start` skill for all of the above.

## Loom Not Discovered

### Symptom

`tie-offs/<rig>/state.json` does not contain your loom.

### Common Causes

1. **Directory name does not end in `-loom`**
   - ❌ `rig/planning/` — not discovered
   - ✅ `rig/planning-loom/` — discovered

2. **Knot files are not `.md` files**
   - ❌ `rig/planning-loom/goals-review.yaml` — not discovered
   - ✅ `rig/planning-loom/goals-review.md` — discovered

3. **Knot files are nested too deep**
   - Knot definitions must be at the **first level** inside the loom
     directory.
   - ❌ `rig/planning-loom/subdir/goals-review.md` — not discovered
   - ✅ `rig/planning-loom/goals-review.md` — discovered

### Fix

Verify the directory name and file locations, then restart Knot so it
re-scans the rig directory.

## Profile Not Found

### Symptom

Knot processing fails with `ProfileNotFound` error. The service log
shows a failure for the affected knot.

### Common Causes

1. **Profile file does not exist** at `rig/profiles/{name}.md`.
2. **Profile name mismatch** — the `agent-profile-ref` in the knot file
   does not match the profile's `name` field or filename stem.
3. **Profile has invalid YAML frontmatter** — Knot cannot parse it.

### Fix

Check the profile file exists and is valid:

```bash
cat rig/profiles/{name}.md
```

Verify the `name` field matches the filename stem.

If the profile is correct, the issue is likely the `agent-profile-ref`
value in the knot file.

## Knot Processing Fails

### Symptom

`tie-offs/<rig>/state.json` shows the knot with status `failed` and a
`last_error` message.

### Diagnostics

1. Check the service log for details:

   ```bash
   grep '[KNOT][EVENT]' tie-offs/<rig>/knot-service.log | grep "knot={knot-name}"
   ```

2. Check the tie-off file — it may contain partial output:

   ```bash
   cat tie-offs/<rig>/{loom-id}/tie-off-{knot-name}.md
   ```

3. Check the service log for timeout events:

   ```bash
   grep 'TimeoutExceeded' tie-offs/<rig>/knot-service.log
   ```

### Common Fixes

| Error | Cause | Fix |
|-------|-------|-----|
| TimeoutExceeded | Agent session exceeded the profile timeout | Increase `timeout` in the profile's frontmatter |
| no final response: … | Agent ended its turn without a final response (abrupt turn-end — a failure, not a timeout). Knot re-enters the session up to 10 times (or the profile timeout budget) asking for the final response; a successful nudge completes the strand transparently | If it still fails, check provider/model health and consider a larger profile `timeout` so the nudge attempts fit the budget. A failed tie-off section was written with the attempt count (`after N attempts`) |
| context limit reached | The session's context exceeded the model window and pi's own compact-and-retry could not recover it — the kept context still does not fit. No retries (re-entry cannot help); a `Failed` tie-off is written and no operational event is recorded (no deadline was breached) | Narrow the prompt/strand scope: smaller strand files, tighter knot instructions, `@file` references instead of inlined content — or use a profile with a larger-window model. Check `ContextCompacted` entries in the service log for frequency (`reason: "overflow"` = limit hit) |
| ProfileNotFound | Profile referenced by knot does not exist | Create the profile file |
| KnotParseWarning | Invalid YAML in knot file | Fix frontmatter syntax |
| Strand dir not found | `strand-dir` points to non-existent directory | Create the directory or fix the path |

## File Watcher Missed an Event

### Symptom

You created or modified a file, but the knot did not trigger.

### Fix

Touch the strand file to generate a fresh filesystem event:

```bash
touch project/prds/my-prd.md
```

Or restart Knot to trigger a full re-scan of the rig directory.

## Knot Oscillates (Keeps Re-running)

### Symptom

The same knot triggers repeatedly without converging. The tie-off file
shows alternating "changes made" and "no changes" entries.

### Cause

Two knots form a feedback loop without a convergence mechanism.

### Fix

Apply loop-breaking patterns from the [Design Guide](design-guide.md):

1. **One-way authority** — designate one knot as authoritative for each
   domain.
2. **Status-gating** — a knot only acts when the strand is in a
   specific status.
3. **Strand acknowledgement** — the knot skips already-processed
   strand content.

## Agent Session Fails Repeatedly

### Symptom

The service log shows multiple `SessionResumed` entries for the same
strand, eventually followed by a failure.

### Cause

The agent invocation keeps failing (network error, provider outage,
model error). Knot retries up to 10 times with 10-second delays.

### Fix

- Check the service log for `TimeoutExceeded` — if the session is too
  slow, increase the profile's `timeout` value.
- Check your LLM provider's status page for outages.
- Verify the agent CLI (`pi`) is working independently:
  `pi --help`

## Knot Stalls — Rig Silent for Minutes (Knot 0.40.0+)

### Symptom

A knot sits in `processing` with no visible work: the session is alive
but silent (a hung bash command, a stalled provider call, a deadlocked
subprocess). The service log shows an `AgentInactivity` event —
`silent_secs`, `window_secs`, the session ID, and the blocked call
when it could be named (e.g. `bash("npm run build")`) — followed by a
`SessionResumed` entry for the restart.

### What Knot Does

The inactivity watchdog (default window **300s**;
`inactivity-timeout-seconds` in `rig/.workspace-agent-config.yaml`,
`0` disables) kills a session that produces no output for the window
and restarts it with a blocking-call note telling the agent to keep
emitting progress (run long tasks in the background and poll, or
stream the output). A healthy outputting command never trips it —
streamed tool output keeps resetting the timer.

### Fix

- If a stall is legitimate (a genuinely quiet long step), raise
  `inactivity-timeout-seconds` and restart Knot.
- If it recurs for the same command, the command is hanging: check it
  independently, or rework the knot so the agent polls a backgrounded
  task instead.
- Use `agent-adapter: pi-json` for blocked-call identification and
  session-resume restarts (`--session-id`); with `pi-stdio` the
  restart is a fresh session and the blocked call is not named.
- Repeated `AgentInactivity` entries ending in `KnotFailed` with a
  `TimeoutExceeded` operational event mean every restart re-hung — the
  note is being ignored or the hang is deterministic; fix the command
  rather than the window.

## Strand Not Being Processed (Binary File)

### Symptom

A file change in the strand directory is not triggering the knot. The
service log shows `StrandIgnored`.

### Cause

The file is detected as binary (contains null bytes in the first 8KB).
Knot only processes text files.

### Fix

Use a text-based file format, or change the knot's `strand-dir` to
watch a directory containing only text files.

## Strand Skipped (File Missing)

### Symptom

The service log shows `StrandSkipped` for a file that should exist.

### Cause

The file was temporarily missing when Knot tried to read it. This can
happen with editors that use atomic writes (write to temp file, then
rename). Known temp files (e.g. macOS `sed -i` temp files) are skipped
silently — unknown missing files produce `StrandSkipped` events.

### Fix

Usually resolves on the next file modification. If persistent, check
that no other process is competing for the file.

## Service Log Is Missing

### Symptom

`tie-offs/<rig>/knot-service.log` does not exist.

### Explanation

Knot never opens that file — it is **appended by the launcher** (the
`knot-start` skill or a `nohup knot >> … &` redirect). If Knot was
started without redirecting its stderr to the service log, no file is
created and the `[KNOT][EVENT]` / `[KNOT][STATE]` lines are lost. This
is normal — but it is also the only record that spans restarts, so
start Knot with the `knot-start` skill.

(Pre-0.41.0 projects may also have legacy `.rig-log` / `.loom-log`
files left in the runtime tree; since plan 083 they are inert — never
read, written, or cleared — and can be deleted.)

## State File Shows Stale Data

### Symptom

`tie-offs/<rig>/state.json` shows outdated processing status.

### Explanation

The state file is written when the state actually changes (a background
writer ticks every 5 seconds, but idle ticks never rewrite it). There is
up to a 5-second delay between an event and its reflection in the state
file.

### Fix

Wait a few seconds and check again, or read the service log (the
`[KNOT][EVENT]` / `[KNOT][STATE]` stderr lines) for real-time events.

---

## Using the Diagnostic Skills

Knot ships with two skills that help diagnose and review rig issues:

### `knot-analyst` — Live rig health and productivity

Use `knot-analyst` to get a structured assessment of your rig's health.
It checks:

- **Operational activity** — timeouts, failures, retries, idle periods
- **Git history** — commit frequency and direction
- **Project progress** — plan completion, phase status
- **Stagnation** — stale strands, loop oscillation
- **Blockers** — timeout walls, missing profiles, dead subscriptions

Run it when:
- The rig seems stuck or unproductive
- You want a health check with a traffic-light score
- You need to identify blockers

### `knot-manage` — Review completed work

Use `knot-manage` to review what the rig has produced:

- **Tie-off quality** — substantive output vs. "no changes needed"
- **Interaction chains** — did producer→consumer communication work?
- **Git commit quality** — are commits meaningful and well-scoped?
- **Communication gaps** — unanswered events, dead subscriptions, oscillation

Run it when:
- You want to assess the quality of the rig's output
- You need to trace why a producer→consumer chain didn't work
- You are reviewing the rig's work after a run
