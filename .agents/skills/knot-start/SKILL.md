---
name: knot-start
description: "Start, stop, restart, and supervise the Knot service the way an agent should: always append the service log to `tie-offs/<rig>/knot-service.log`, run it in the background with a recorded PID, verify startup through `tie-offs/<rig>/state.json`, and stop it with SIGINT so the queue drains. Covers rig auto-discovery, multiple rigs, hot-reload vs restart-required changes, service-log inspection and rotation. USE FOR: start knot, start the service, run knot, background knot, restart knot, stop knot, kill knot, service log, knot-service.log, knot pid, is knot running, service not running, verify knot started, daemon, rig not running, knot crashed, service died. DO NOT USE FOR: initialising rig files (use knot-init), creating looms/knots/profiles (use knot-create), triggering work (use knot-dispatch), inspecting rig state (use knot-inspect), analysing productivity (use knot-analyst)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.39.0+"
---

# Knot Start Skill

Run the Knot service from an agent session: start it in the background
**with its output appended to a durable log**, verify it came up, stop
it cleanly, and restart it when a change needs a fresh boot.

**Service log:** `tie-offs/<rig>/knot-service.log` — the appended
stderr/stdout of the service, **never cleared by Knot** (this skill
creates it; Knot itself writes only to stderr).
**PID file:** `tie-offs/<rig>/knot-service.pid` — written by *this*
skill; Knot writes no pidfile of its own.
**State file:** `tie-offs/<rig>/state.json` — written when the state
changes (the state writer ticks every 5 seconds; identical snapshots
are never rewritten, so an unchanged mtime means the rig is idle).

---

## Core Philosophy

### The Service Log Is the Only Cross-Run Record

Knot's run activity is **in-memory per process** (Knot 0.41.0+ — the
retired `.rig-log` / `.loom-log` JSONL files are gone, and nothing is
cleared at startup). The structured activity — every domain event as a
`[KNOT][EVENT]` line and every `state.json` write as a `[KNOT][STATE]`
line — goes to **stderr only**, and so does everything else Knot prints
— startup warnings, `[startup] loaded N persisted event(s)`, `[queue]
repaired …` notices, panics. None of it survives the terminal closing
unless stderr was captured. `knot-service.log` is that capture, and it
is the reason to start Knot through this skill instead of a bare
`cargo run`.

### The `[KNOT][…]` Trace Exists Nowhere Else

Knot's activity trace goes to **stderr only** — it is not written to any
file (the retired `.rig-log` / `.loom-log` are gone, and nothing is
cleared or appended to them at startup):

```
[2026-09-02T22:12:47+10:00] [KNOT][WATCH] register /path/to/rig (type=Rig)
[2026-09-02T22:12:47+10:00] [KNOT][CONFIG] git_versioner — initialised rig git repository
```

(Real lines from a live start — local-time timestamps with offset, so
they sort correctly against `state.json`'s `updated_at`.)

| Tag | Covers |
|-----|--------|
| `[KNOT][EVENT]` | **one line per domain event** (Knot 0.41.0+): loom lifecycle (`LoomStarted`, `KnotRegistered`, …), strand processing (`KnotProcessing`, `KnotCompleted`, `KnotFailed`, `StrandProcessed`, …), `SessionResumed`, `TimeoutExceeded`, `AgentInactivity`, `QueueIdle` — fields as `key=value` pairs (`loom=`, `knot=`, `strand=`, `tie-off=`, `error=`, …) |
| `[KNOT][STATE]` | **one line per actual `state.json` write** (Knot 0.41.0+): `initial snapshot …` at startup, then `change` deltas (loom/knot/profile add/remove, field changes, queue add/drain) |
| `[KNOT][NOTIFY]` | raw filesystem event → mapped strand/config event (did the watcher see the file at all?) |
| `[KNOT][STRAND]` | strand events being processed |
| `[KNOT][KNOT]` | knot register / unregister / watcher changes for a knot |
| `[KNOT][LOOM]` | loom register / unregister / discover |
| `[KNOT][CONFIG]` | config-pipeline activity — in practice `git_versioner — …`: rig git init, project `.gitignore` exclusion, and the commit / skip / failure for each agent turn |
| `[KNOT][WATCH]` | watch / unwatch per path (`type=Rig`, `Strand`, …) |

So "the knot did not fire" is usually answered by `[KNOT][EVENT]`,
`[KNOT][NOTIFY]` and `[KNOT][WATCH]` lines — and those are recoverable
only if the run was redirected. This is the strongest reason to always
append.

### Always Append, Never Truncate

Use `>>`, never `>`, and never `truncate`/`: >` the log. The file is the
rig's service history across boots; the agent that truncates it destroys
the only evidence of the previous failure. Shrink it by **renaming**
(rotate), not by clearing — see "Supervising the Log".

### One Service Per Rig, One Log Per Rig

Starting a second Knot on the same rig is not harmless: both instances
watch the same strand directories, and both rewrite `state.json` when
their own state changes (their `[STATE]` lines interleave in any shared
capture). Always check for a live instance before starting (step 2).

Rig switching (`knot dev-rig`) means several services can run at once in
one project — so the log lives **inside the rig's runtime tree**, not at
a single project-root path. A shared `./knot-service.log` would
interleave two rigs' output and make both unreadable.

### The Agent Owns the Process — and Its Signals

Knot is started as a background process so the agent's tool call
returns; the skill records the PID so it can be stopped later.

**Knot handles SIGINT only** (its Ctrl+C handler). There is no SIGTERM
handler:

- `kill -INT <pid>` → graceful cascade: queue drains, `LoomStopped` is
  recorded on the service log, the process exits.
- `kill -TERM <pid>` / `kill -9 <pid>` → immediate death: no drain, no
  `LoomStopped`, an in-flight strand left mid-run.

So **stop Knot with `-INT`**, never with a plain `kill` (which is
SIGTERM). The recovery from `-9` is survivable — the event queue is
disk-backed (`tie-offs/<rig>/events/*.json`), so an interrupted strand's
event file survives and is re-queued on next start, and knots are
designed to be idempotent — but it is still the last resort.

### Launch From the Project Root

Rig discovery is CWD-based: zero `*-rig` directories → `rig/` is created
and used; one → used; several → error. Start Knot from the **project
root**, never from inside `rig/` or a subdirectory, or it will look at
the wrong place (or create a stray `rig/`).

---

## What the Service Writes

| Path | Written by | Content | Lifetime |
|------|-----------|---------|----------|
| `tie-offs/<rig>/knot-service.log` | the launcher (**this skill**) | the service's stderr/stdout — single-line `[KNOT][EVENT]` / `[KNOT][STATE]` records plus startup/shutdown traces | **append across runs** |
| `tie-offs/<rig>/knot-service.pid` | this skill | background PID | until stopped |
| `tie-offs/<rig>/state.json` | Knot (on state change) | rig snapshot (looms, knots, profiles) | rewritten only when the state actually changes |
| `tie-offs/<rig>/events/*.json` | Knot | pending event queue | survives restarts |

(Pre-0.41.0 Knots also wrote `tie-offs/<rig>/.rig-log` and
`tie-offs/<rig>/{loom-id}/.loom-log` JSONL files; since plan 083 those
are gone — run activity is in-memory per process and the service's
stderr is the log. Any legacy files left by a 0.31.0 migration are
inert and may be deleted.)

---

## Prerequisites

1. Knot binary available — installed (`cargo install --path .` → `knot`)
   or built in the Knot source tree (`./target/debug/knot`).
2. The rig is initialised (see `knot-init`). A missing rig is *not* an
   error for the default rig — Knot creates `rig/` on start — but an
   agent should expect `knot-init` to have run first.
3. CWD is the project root (the directory containing `rig/`).

---

## Agent Workflow — Start the Service

### 1. Resolve the rig and its runtime root

| Launch | Rig dir | Runtime root (log goes here) |
|--------|---------|------------------------------|
| `knot` (default) | `rig/` | `tie-offs/rig/` |
| `knot dev-rig` | `dev-rig/` | `tie-offs/dev-rig/` |

```bash
RIG=rig                                   # or: dev-rig
RUNTIME=tie-offs/$RIG
mkdir -p "$RUNTIME"
```

If several `*-rig` directories exist and no rig name was given, Knot
exits with a "multiple rigs found" error — ask the user which rig to
run rather than guessing.

### 2. Check for a live instance first

```bash
# Fresh state file (written within the last 10s) means the service is up
python3 - "$RUNTIME/state.json" << 'EOF'
import json, sys, datetime, pathlib
p = pathlib.Path(sys.argv[1])
if not p.exists():
    print("not running (no state.json)"); sys.exit(0)
age = (datetime.datetime.now(datetime.timezone.utc)
       - datetime.datetime.fromisoformat(
             json.load(open(p))["updated_at"].replace("Z", "+00:00"))
       ).total_seconds()
print("running" if age < 10 else f"stale by {age:.0f}s (crashed?)")
EOF

# And/or the recorded PID
[ -f "$RUNTIME/knot-service.pid" ] && kill -0 "$(cat "$RUNTIME/knot-service.pid")" 2>/dev/null \
  && echo "pid $(cat "$RUNTIME/knot-service.pid") alive"
```

- **Running** → report it (pid, log path, loom/profile counts) and do
  **not** start a second instance. Use "Restart" below if the user asked
  for a restart.
- **Stale state file, no live pid** → the service died. Read the tail of
  the service log **before** starting again — that is the whole point of
  keeping it:
  ```bash
  tail -40 "$RUNTIME/knot-service.log"
  ```

### 3. Keep the log out of git (idempotent)

The runtime tree is committed with the project, but the service log
churns on every line of stderr. Add both patterns to the project's
`.gitignore` if absent (a bare name with no `/` matches in every
directory, so one entry covers all rigs):

```bash
for pat in knot-service.log knot-service.pid; do
  grep -qxF "$pat" .gitignore 2>/dev/null || echo "$pat" >> .gitignore
done
```

Do **not** ignore `tie-offs/` as a whole — tie-offs, logs, and the event
queue are project history by design.

### 4. Start it, appending to the service log

Installed binary:

```bash
nohup knot >> "$RUNTIME/knot-service.log" 2>&1 &
echo $! > "$RUNTIME/knot-service.pid"
```

**`echo $!` must be the very next statement after the backgrounded
command.** If the launch is chained with `&&` (e.g.
`cd proj && nohup knot >> log 2>&1 &`), the `&` backgrounds the whole
chain and `$!` is the *wrapper shell* — the pidfile then points at a bash
process and the real Knot process is left uncontrollable (observed:
`ps -o comm= -p $(cat …/knot-service.pid)` returned `bash`, and
`kill -INT` on it left Knot running). Confirm the pid is Knot right after
starting:

```bash
ps -o pid=,comm= -p "$(cat "$RUNTIME/knot-service.pid")"   # expect: knot
```

From the Knot source tree (build first — `nohup cargo run` rebuilds
inside the log and hides compile errors behind service noise):

```bash
cargo build --quiet
nohup ./target/debug/knot >> "$RUNTIME/knot-service.log" 2>&1 &
echo $! > "$RUNTIME/knot-service.pid"
```

Named rig (runtime root must match the rig name):

```bash
RUNTIME=tie-offs/dev-rig; mkdir -p "$RUNTIME"
nohup knot dev-rig >> "$RUNTIME/knot-service.log" 2>&1 &
echo $! > "$RUNTIME/knot-service.pid"
```

`nohup` detaches the process from the agent's shell (SIGHUP-safe); the
redirect is what makes the run debuggable after the fact.

### 5. Verify it came up

Wait for a fresh state file (Knot writes the baseline immediately at
boot, then only when the state changes — so a fresh `updated_at` is
proof of a recent start, not of liveness in steady state):

```bash
for i in $(seq 1 15); do
  if python3 - "$RUNTIME/state.json" << 'EOF'
import json, sys, datetime, pathlib
p = pathlib.Path(sys.argv[1])
if not p.exists():
    sys.exit(1)
age = (datetime.datetime.now(datetime.timezone.utc)
       - datetime.datetime.fromisoformat(
             json.load(open(p))["updated_at"].replace("Z", "+00:00"))
       ).total_seconds()
sys.exit(0 if age < 10 else 1)
EOF
  then echo "up"; break; fi
  sleep 1
done
```

Then confirm the process and read the new log lines — a clean boot shows
discovery output and no `WARNING`/`Error` lines:

```bash
kill -0 "$(cat "$RUNTIME/knot-service.pid")" && echo alive
tail -20 "$RUNTIME/knot-service.log"
grep -nE 'WARNING|Error|panic' "$RUNTIME/knot-service.log" | tail -20
python3 -c "import json;s=json.load(open('$RUNTIME/state.json'));print(s['rig_path'],len(s['looms']),'looms',len(s['profiles']),'profiles')"
```

### 6. Report

Tell the user: pid and pidfile, `tail`-able log path, rig path from
`state.json`, loom/knot and profile counts, and anything the log shows
(warnings, repaired queue entries, no looms discovered).

---

## Agent Workflow — Stop the Service

```bash
PID=$(cat "$RUNTIME/knot-service.pid")
ps -o pid=,comm= -p "$PID"             # sanity check: must be `knot`
kill -INT "$PID"                       # SIGINT = the graceful path
for i in $(seq 1 15); do
  kill -0 "$PID" 2>/dev/null || { echo "stopped after ${i}s"; break; }
  sleep 1
done
kill -0 "$PID" 2>/dev/null && \
  echo "still up after 15s — an in-flight knot run can outlast the drain; check the log before escalating"
rm -f "$RUNTIME/knot-service.pid"
```

Expect **5–6 seconds**, not instant: the drain has a 5-second timeout
safety net, and an idle service normally prints
`WARNING: pipeline tasks did not drain within 5s, aborting` on the way
out. That warning at shutdown is routine — nothing is lost (the queue is
on disk); at worst the `LoomStopped` event line is skipped.

Verify the graceful finish in the service log (the shutdown entries are
the last lines of the captured stderr):

```bash
grep -h LoomStopped "$RUNTIME/knot-service.log" 2>/dev/null | tail
tail -5 "$RUNTIME/knot-service.log"
```

If the process ignores SIGINT for a long time (a single agent run can
take minutes — the default profile timeout is 300s), wait rather than
escalate. `kill -9` is the last resort: the queue drains not at all, no
`LoomStopped` is written, and the interrupted strand's event file stays
in `tie-offs/<rig>/events/` to be re-queued and re-run on next start
(that is why every knot must be idempotent).

No pidfile but a Knot process is running (started from a human terminal,
`cargo run`, etc.)? Find it, identify it, then SIGINT it:

```bash
pgrep -x knot                       # the binary is named `knot`
ls -l /proc/<pid>/cwd               # which project does this service own?
kill -INT <pid>
```

**Never `pkill knot`, and do not trust `pgrep -f knot`.** A workstation
usually runs one Knot service **per project** (observed: an unrelated
project's service with 6 hours of uptime alongside the one under work), so
a broad match stops somebody else's rig. `pgrep -f` is worse still — it
matches the agent's own shell command line and any `knot step` process.
Match `pgrep -x knot`, then confirm each candidate by the working
directory it owns before signalling it.

---

## Agent Workflow — Restart

```bash
# stop (above), then start
nohup knot >> "$RUNTIME/knot-service.log" 2>&1 &
echo $! > "$RUNTIME/knot-service.pid"
```

Then re-verify registration (step 5) and confirm the change landed —
`KnotRegistered` / `LoomStarted` entries for the affected loom:

```bash
grep "loom={loom-id}" "$RUNTIME/knot-service.log" | tail -20
```

**Restart is not always needed:**

| Change | Restart? |
|--------|----------|
| Strand file created/changed | No — file watcher fires |
| New knot/loom directory | Yes — watches are registered at startup |
| Knot definition `.md` edited | Yes if the file moved/renamed; content edits are re-read on the next event |
| Profile `.md` edited | No — profiles resolve at processing time |
| `rig/models.yml` alias swapped | No — registry resolved fresh every run |
| `rig/.workspace-agent-config.yaml` `agent-adapter` | **Yes** — adapter chosen at startup |
| Knot binary updated | Yes (and check the `knot-update` skill for migrations) |

---

## Supervising the Log

```bash
# Follow along while triggering work
tail -f "$RUNTIME/knot-service.log"

# What went wrong at the last boot?
grep -nE 'WARNING|Error|panic' "$RUNTIME/knot-service.log" | tail -20

# Which line did the last start begin at? (mark, then restart)
wc -l < "$RUNTIME/knot-service.log"
```

The log mixes the structured `[KNOT][SUBSYSTEM]` trace (tag table above)
with the service's own startup/pipeline lines:

| Log line | Meaning |
|----------|---------|
| `[KNOT][NOTIFY] Created path → knot-name` | the watcher saw a strand — first thing to check when a knot "did not fire" |
| `[KNOT][WATCH] unwatch path` | a watch went away (knot/loom removed or renamed) |
| `[startup] loaded N persisted event(s) from disk` | N queued events re-queued from a previous run — expected after a crash or a `knot step` |
| `[startup] migration: …` | legacy runtime layout moved to `tie-offs/<rig>/` |
| `[queue] repaired …` | a knot file's queue identity was rewritten — read it, it reports a repaired definition |
| `[pipeline] QueueIdle …` | queue idle watchdog fired |
| `[state-writer] write failed` | runtime root not writable — service is up but state is stale |
| `WARNING: rig git init` | `rig/.git` could not be created — rig versioning is off |
| `WARNING: pipeline tasks did not drain within 5s, aborting` | printed during shutdown (the 5s drain safety net) — routine |
| `Background task failed: …` | a pipeline task died; the service may keep writing state while doing nothing |

**Rotation** (append-only means it grows; rotate by rename, never by
truncating the live file — an appended-redirect keeps its offset and a
cleared file would silently swallow the next lines):

```bash
mv "$RUNTIME/knot-service.log" "$RUNTIME/knot-service.log.1"   # then restart
```

Keep at most one rotated file and delete older ones; the log is debug
history, not an audit record (tie-offs and git commits are the audit
record).

**Stepping:** when walking the queue one event at a time with
`knot step` (service stopped — see `knot-dispatch`), send its output to
the same log so the cross-run record stays single-source:

```bash
set -o pipefail                        # keep Knot's exit code
knot step 2>&1 | tee -a "$RUNTIME/knot-service.log"
```

---

## Foreground Runs (humans only)

`knot` in a terminal, stopped with Ctrl+C, is fine for a human
debugging a boot failure — but an agent must never run the service in
the foreground: the tool call blocks until the process exits, and the
output is lost when the session ends. If a foreground service is found
and must be controlled, restart it through this skill.

---

## Error Handling

| Scenario | Action |
|----------|--------|
| `state.json` missing right after start | `tail -40` the service log — usually a rig-discovery error or an unwritable path |
| "multiple rigs found" | Ask the user which rig; launch with `knot <rig-name>` |
| `knot: command not found` | Build/install (`cargo build` in the Knot tree, or `cargo install --path .`) and use `./target/debug/knot` |
| pidfile exists, pid dead | Service died — read the log tail, then restart (keep the log; do not truncate) |
| pidfile pid alive but `comm` is not `knot` | `$!` was captured after a `&&` chain, so it recorded the wrapper shell. Find the real service (`pgrep -x knot` + `ls -l /proc/<pid>/cwd`), SIGINT the wrapper's children as needed, and relaunch with `echo $!` on its own line |
| state file stale but pid alive | Likely mid long run or the rig is idle (the state file is now change-driven) — check the service log before assuming a crash |
| Log is empty after a start | Wrong runtime root (wrong CWD or rig name), or the service was started in the foreground |
| Two Knots running on one rig | Stop both with `kill -INT`, then start one; the two services were interleaving state writes and event lines |
| Port/HTTP questions | Knot has no HTTP interface in 0.3x — control and observation are files; see `knot-inspect` |

---

## Quick Reference

```bash
# Start (append the service log, record the pid)
RUNTIME=tie-offs/rig; mkdir -p "$RUNTIME"
nohup knot >> "$RUNTIME/knot-service.log" 2>&1 &
echo $! > "$RUNTIME/knot-service.pid"
ps -o pid=,comm= -p "$(cat "$RUNTIME/knot-service.pid")"   # must print `knot`

# Was it started recently? (baseline write at boot; the pidfile — not
# state.json freshness — is the liveness probe in steady state)
cat tie-offs/rig/state.json | python3 -c "import sys,json;print(json.load(sys.stdin)['updated_at'])"
kill -0 "$(cat tie-offs/rig/knot-service.pid)" && echo "process alive"

# Follow the service log
tail -f tie-offs/rig/knot-service.log

# Stop gracefully (SIGINT — never a plain kill; takes ~5-6s to drain)
kill -INT "$(cat tie-offs/rig/knot-service.pid)" && rm -f tie-offs/rig/knot-service.pid

# Which project does a running Knot own? (never `pkill knot` — one rig per project)
for p in $(pgrep -x knot); do echo "$p $(readlink /proc/$p/cwd)"; done

# Restart
kill -INT "$(cat tie-offs/rig/knot-service.pid)"; sleep 6
nohup knot >> tie-offs/rig/knot-service.log 2>&1 &
echo $! > tie-offs/rig/knot-service.pid

# Named rig
RUNTIME=tie-offs/dev-rig; mkdir -p "$RUNTIME"
nohup knot dev-rig >> "$RUNTIME/knot-service.log" 2>&1 &
echo $! > "$RUNTIME/knot-service.pid"

# Keep the logs out of git
grep -qxF knot-service.log .gitignore || echo knot-service.log >> .gitignore
```

---

## Cross-Reference

- **knot-init** — creates the rig files and installs the skills; checks
  for a running service the same way (fresh `state.json`) and defers
  starting it to this skill.
- **knot-dispatch** — `knot step` processes one queued event and must
  run with the service stopped.
- **knot-inspect** — rig/loom/knot/profile state from `state.json` and
  the per-run logs.
- **knot-analyst** — productivity, blocker, and health assessment; the
  service log adds the cross-run view the per-run logs cannot give.
- **knot-update** — what to migrate when the Knot binary version
  changes (always restart after a migration).
