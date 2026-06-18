# Troubleshooting

Common issues and how to resolve them.

## Knot Is Not Running

### Symptom

```bash
curl http://localhost:3000/health
# curl: (7) Connection refused
```

### Fix

Start Knot from your project directory:

```bash
cargo run
# or, if installed:
knot
```

## Loom Not Discovered

### Symptom

`GET /looms` does not return your loom.

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

Verify the directory name and file locations, then trigger a manual
rescan:

```bash
curl -X POST http://localhost:3000/config/reload
```

## Profile Not Found

### Symptom

Knot processing fails with `ProfileNotFound` error. The activity log
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

Verify the `name` field matches:

```bash
curl http://localhost:3000/profiles/{name}
```

If the profile is returned, the issue is likely the `agent-profile-ref`
value in the knot file.

## Knot Processing Fails

### Symptom

`GET /looms/{id}/knots/{name}` returns status `failed` with a
`last_error` message.

### Diagnostics

1. Check the activity log for details:

   ```bash
   curl http://localhost:3000/looms/{id}/activity
   ```

2. Check the tie-off file — it may contain partial output or error
   details:

   ```bash
   cat rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md
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

Manually re-scan the rig:

```bash
curl -X POST http://localhost:3000/config/reload
```

Or touch the strand file to generate a fresh filesystem event:

```bash
touch project/prds/my-prd.md
```

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

## API Returns 404 for Loom or Knot

### Symptom

```bash
curl http://localhost:3000/looms/my-loom
# 404 Not Found
```

### Fix

1. List available looms to verify names:

   ```bash
   curl http://localhost:3000/looms
   ```

2. The loom ID includes the `-loom` suffix. Use the full name:

   ```bash
   # If the directory is rig/my-loom/
   # The loom ID is "my-loom" (not "my")
   curl http://localhost:3000/looms/my-loom
   ```

## Rig-Log or Loom-Log Is Missing

### Symptom

`rig/.rig-log` or `rig/tie-offs/{loom-id}/.loom-log` does not exist.

### Explanation

These files are created when events occur. An empty rig with no
processing activity will not have log files yet. This is normal.

The rig-log is created on the first `TimeoutExceeded` or `QueueIdle`
event. The loom-log is created when the loom starts processing.
