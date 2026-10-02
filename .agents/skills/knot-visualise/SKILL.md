---
name: knot-visualise
description: "Visualise the whole-rig producer→consumer topology as a self-contained HTML file. Runs the rig graph extractor (`scripts/rig-graph.py`), which pulls the rig's knots (nodes) and strand connections (edges — `event:` subscriptions and filesystem `strand-dir`s), merges the result into a D3 HTML template, and saves it to a user-specified `--out` location. USE FOR: visualise rig, rig graph, rig visualisation, rig topology, knot graph, graph rig, show rig structure, rig map, rig diagram, knot connections, who subscribes to what, event flow diagram, rig overview, visual rig, export rig graph, rig HTML. DO NOT USE FOR: inspecting raw rig state (use knot-inspect), creating looms or knots (use knot-create), triggering knots (use knot-dispatch), analysing rig productivity (use knot-analyst)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.41.0+ (reads rig files and state.json; service need not be running)"
---

# Knot Visualise Skill

Render the whole-rig graph — knots, looms, and their event/strand
connections — into a single HTML file the user can open in a browser.
The output is **fully self-contained** (D3 is inlined): no network needed
to view it, safe to share.

**The extractor does not need the Knot service running** — it reads the
rig's `.md` files, plus `tie-offs/<rig>/state.json` when present.

---

## The script

`scripts/rig-graph.py` (stdlib-only Python 3, lives next to the rig's
project root — in a Knot-managed project, `scripts/rig-graph.py` relative
to the current working directory).

```
python3 scripts/rig-graph.py --out <path.html> [--rig <dir>] [--no-state] [--json]
```

| Flag | Meaning |
|------|---------|
| `--out <path.html>` | **Required** (unless `--json`). Output HTML location — **ask the user** where they want it; parent directories are created if missing. |
| `--rig <dir>` | Rig directory, relative to CWD. Default `rig`. |
| `--no-state` | Omit the runtime state overlay (node status / last activity). By default the overlay is applied only when `tie-offs/<rig>/state.json` exists. |
| `--json` | Print the graph JSON to stdout and skip HTML — for inspecting the graph structure without rendering. |

Exit code `2` with a message on stderr when the rig dir is missing or no
looms are found.

---

## Workflow

1. **Ask for the output location** (`--out`). Suggest a sensible default
   (e.g. `project/rig-graph.html`) if the user has no preference, but the
   path is theirs to choose.
2. **Run** `python3 scripts/rig-graph.py --out <path>` from the project
   root. Confirm the `wrote <path> (N nodes, M edges)` line.
3. **Tell the user** the path and what to expect (a force-directed graph;
   drag to rearrange, scroll to zoom, hover a node for metadata).
4. Only if debugging: `--json` prints the raw graph; `--no-state` gives a
   pure topology view without runtime status.

---

## Graph-model vocabulary

**Nodes**

| Kind | What it is |
|------|------------|
| `knot` | One knot (`.md` file in a loom dir, dirs ending `-loom`). Coloured by loom; label is the knot id. |
| `input` | A filesystem `strand-dir` (e.g. `project/prds`) — the rig's external entry points. Dimmed diamonds. |
| `system` | The synthetic `Knot (system)` node — source of rig-level `event:knot:<Event>` subscriptions (Knot engine events). |
| `unresolved` | A subscription target (knot or loom) that **does not exist** in the rig — dashed red, surfaced deliberately, never dropped. |

Knot ids are unique per rig; when the same knot id exists in **two looms**,
node ids become `<loom>:<knot>` (labels stay bare).

**Edges** (derived from each knot's single `strand-dir`)

| `strand-dir` form | Edge(s) |
|---|---|
| `event:<knot>:<Event>` | producer knot → this knot (a bare knot id matching knots in several looms fans out to all of them) |
| `event:<*-loom>:<Event>` | every knot in that loom → this knot |
| `event:*:<Event>` | every knot in the rig → this knot |
| `event:knot:<Event>` | system node → this knot |
| plain path | input node → this knot |

Edges are labelled with the event name (or path). `unresolved` edges are
dashed.

**State overlay** (only when `state.json` exists and `--no-state` is not
given): each knot node carries `status` (`idle` / `processing` /
`completed`) and `last_event_at`, shown in the hover tooltip.

---

## Interpreting the graph

- **Loops** — a chain of event edges that returns to its starting knot:
  expected feedback (producer/consumer review) but verify against the
  design; a knot re-triggering *itself* is a bug (Knot self-excludes
  terminal events, but design-level ping-pong still shows).
- **Fan-out** — a knot (or loom) many edges leave: shared producers are
  good; a *wildcard* consumer with dozens of incoming edges is a smell
  (over-broad subscription).
- **Orphaned inputs** — `input` nodes no one seems to reach, or knots with
  no outgoing event edges that are not terminal producers: possibly
  dead or manually-triggered work.
- **Unresolved targets** — dangling subscriptions: a producer was renamed
  or deleted, or the subscription is a typo. Check the knot file.
- **Terminal producers** — knots with no event subscriptions at all
  (plain `strand-dir` only): the rig's entry points; verify one exists per
  intended trigger.

---

## Troubleshooting

| Symptom | Check |
|---------|-------|
| `error: rig directory not found` | Run from the project root, or pass `--rig`. |
| `error: no looms found` | Loom dirs must end in `-loom` under the rig dir. |
| Graph looks empty | Try `--json` and inspect; confirm the knot `.md` files have `name` and `strand-dir` frontmatter. |
| Node shows no status | Service not running (no `state.json`) or `--no-state` — expected, not an error. |
| Duplicate-looking knot labels | Knot id exists in multiple looms — node ids are `<loom>:<knot>`; the tooltip shows the loom. |
