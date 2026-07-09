# Troubleshooting

Common issues and how to resolve them.

## Knot Is Not Running

### Symptom

`rig/state.json` does not exist or is not being updated.

### Fix

Start Knot from your project directory:

```bash
cargo run
# or, if installed:
knot
```

Verify by watching the state file:

```bash
watch -n 2 'cat rig/state.json | python3 -m json.tool'
```

## Loom Not Discovered

### Symptom

`rig/state.json` does not contain your loom.

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

Knot processing fails with `ProfileNotFound` error. The loom-log
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

`rig/state.json` shows the knot with status `failed` and a
`last_error` message.

### Diagnostics

1. Check the loom-log for details:

   ```bash
   cat rig/tie-offs/{loom-id}/.loom-log
   ```

2. Check the tie-off file — it may contain partial output:

   ```bash
   cat rig/tie-offs/{loom-id}/{knot-name}-tie-off.md
   ```

3. Check the rig-log for timeout events:

   ```bash
   cat rig/.rig-log | grep TimeoutExceeded
   ```

### Common Fixes

| Error | Cause | Fix |
|-------|-------|-----|
| TimeoutExceeded | Agent session exceeded the profile timeout | Increase `timeout` in the profile's frontmatter |
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

The loom-log shows multiple `SessionResumed` entries for the same
strand, eventually followed by a failure.

### Cause

The agent invocation keeps failing (network error, provider outage,
model error). Knot retries up to 10 times with 10-second delays.

### Fix

- Check the rig-log for `TimeoutExceeded` — if the session is too
  slow, increase the profile's `timeout` value.
- Check your LLM provider's status page for outages.
- Verify the agent CLI (`pi`) is working independently:
  `pi --help`

## Strand Not Being Processed (Binary File)

### Symptom

A file change in the strand directory is not triggering the knot. The
loom-log shows `StrandIgnored`.

### Cause

The file is detected as binary (contains null bytes in the first 8KB).
Knot only processes text files.

### Fix

Use a text-based file format, or change the knot's `strand-dir` to
watch a directory containing only text files.

## Strand Skipped (File Missing)

### Symptom

The loom-log shows `StrandSkipped` for a file that should exist.

### Cause

The file was temporarily missing when Knot tried to read it. This can
happen with editors that use atomic writes (write to temp file, then
rename). Known temp files (e.g. macOS `sed -i` temp files) are skipped
silently — unknown missing files produce `StrandSkipped` events.

### Fix

Usually resolves on the next file modification. If persistent, check
that no other process is competing for the file.

## Rig-Log or Loom-Log Is Missing

### Symptom

`rig/.rig-log` or `rig/tie-offs/{loom-id}/.loom-log` does not exist.

### Explanation

These files are created when events occur. An empty rig with no
processing activity will not have log files yet. This is normal.

The rig-log is created on the first `TimeoutExceeded` or `QueueIdle`
event. The loom-log is created when the loom starts processing.

## State File Shows Stale Data

### Symptom

`rig/state.json` shows outdated processing status.

### Explanation

The state file is written every 5 seconds. There is up to a 5-second
delay between an event and its reflection in the state file.

### Fix

Wait a few seconds and check again, or read the loom-log directly for
real-time events.
