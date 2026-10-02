#!/usr/bin/env python3
"""Rig graph extractor.

Extracts the rig's knot/strand graph (knots as nodes, event subscriptions and
filesystem strand-dirs as edges) from the rig's .md files and renders it into
the D3 HTML template, writing the result to --out.

Stdlib only. Does not require the Knot service to be running.

Usage:
  python3 scripts/rig-graph.py --out /path/to/graph.html [--rig rig] [--no-state] [--log knot-service.log]
  python3 scripts/rig-graph.py --json [--rig rig]   # print graph JSON, skip HTML
"""

import argparse
import datetime
import json
import os
import re
import sys

TEMPLATE_NAME = "rig-graph-template.html"
D3_VENDOR_NAME = os.path.join("vendor", "d3.min.js")
GRAPH_PLACEHOLDER = "/*__GRAPH_JSON__*/"
D3_PLACEHOLDER = "/*__D3__*/"


def parse_frontmatter(path):
    """Parse flat `key: value` YAML frontmatter from a .md file.

    Returns a dict (empty if the file has no frontmatter). Values are
    unquoted. Lines without a colon and values with `:` after the first
    are taken verbatim after the first colon (event URIs contain colons).
    """
    with open(path, encoding="utf-8") as f:
        text = f.read()
    m = re.match(r"\A---\s*\n(.*?)\n---\s*(\n|\Z)", text, re.DOTALL)
    if not m:
        return {}
    fields = {}
    for line in m.group(1).splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if ":" not in line:
            continue
        key, _, value = line.partition(":")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        fields[key.strip()] = value
    return fields


def is_event_uri(value):
    return value.startswith("event:")


def event_parts(value):
    """`event:<target>:<EventId>` -> (target, event_id)."""
    rest = value[len("event:"):]
    target, _, event_id = rest.partition(":")
    return target, event_id


def scan_rig(rig_dir):
    """Walk the rig: looms (dirs ending -loom) and their knots (.md files)."""
    looms = {}
    for entry in sorted(os.listdir(rig_dir)):
        path = os.path.join(rig_dir, entry)
        if not (os.path.isdir(path) and entry.endswith("-loom") and not entry.startswith(".")):
            continue
        knots = []
        for name in sorted(os.listdir(path)):
            if not (name.endswith(".md") and not name.startswith(".")):
                continue
            fields = parse_frontmatter(os.path.join(path, name))
            knot_id = fields.get("name") or os.path.splitext(name)[0]
            knots.append({
                "id": knot_id,
                "loom": entry,
                "profile": fields.get("agent-profile-ref", ""),
                "strand_dir": fields.get("strand-dir", ""),
                "event_description": fields.get("event-description", ""),
            })
        looms[entry] = knots
    return looms


def load_state_overlay(cwd, rig_dir):
    """Return {(loom_id, knot_id): {status, last_event_at}} from state.json,
    or {} when the file is absent."""
    rig_basename = os.path.basename(os.path.normpath(rig_dir))
    state_path = os.path.join(cwd, "tie-offs", rig_basename, "state.json")
    if not os.path.isfile(state_path):
        return {}
    with open(state_path, encoding="utf-8") as f:
        state = json.load(f)
    overlay = {}
    for loom in state.get("looms", []):
        for knot in loom.get("knots", []):
            info = {}
            if "status" in knot:
                info["status"] = knot["status"]
            if "last_event_at" in knot:
                info["last_event_at"] = knot["last_event_at"]
            if info:
                overlay[(loom["id"], knot["id"])] = info
    return overlay


def build_graph(looms, state_overlay):
    """Build {"nodes": [...], "edges": [...]} from the rig scan."""
    all_knots = [k for ks in looms.values() for k in ks]
    ids = {}
    for k in all_knots:
        ids.setdefault(k["id"], []).append(k)

    def uid(knot):
        """Unique node id for a knot: bare id when unique across looms,
        else loom-prefixed."""
        return knot["id"] if len(ids[knot["id"]]) == 1 \
            else "%s:%s" % (knot["loom"], knot["id"])

    nodes = []
    edges = []
    input_seen = {}   # path -> node id

    for loom_id, knots in looms.items():
        for knot in knots:
            node = {
                "id": uid(knot),
                "kind": "knot",
                "label": knot["id"],
                "loom": loom_id,
                "profile": knot["profile"],
                "strand_dir": knot["strand_dir"],
            }
            if (loom_id, knot["id"]) in state_overlay:
                node.update(state_overlay[(loom_id, knot["id"])])
            nodes.append(node)

            sd = knot["strand_dir"]
            if not sd:
                continue
            if is_event_uri(sd):
                target, event_id = event_parts(sd)
                unresolved = False
                scope = None
                if target == "*":
                    producers = [uid(k) for k in all_knots]
                    scope = "all"
                elif target == "knot":
                    producers = [__SYSTEM_NODE__]
                elif target in looms:
                    producers = [uid(k) for k in looms[target]]
                    scope = "loom"
                elif target in ids:
                    # A bare knot-id subscription matches every knot with
                    # that id (Knot matches on id alone) — may span looms.
                    producers = [uid(k) for k in ids[target]]
                else:
                    # Target knot/loom does not exist in the rig: flag it,
                    # don't drop it.
                    producers = [target]
                    unresolved = True
                if not producers:
                    producers = [target]
                for p in producers:
                    edge = {"source": p, "target": uid(knot), "label": event_id}
                    if scope:
                        # Wide subscription: the edge is really "subscriber
                        # listens to the whole loom / everything" — mark it
                        # so it never reads as a knot-specific send.
                        edge["scope"] = scope
                    if unresolved:
                        edge["unresolved"] = True
                    edges.append(edge)
            else:
                if sd not in input_seen:
                    node_id = "input:%s" % sd
                    input_seen[sd] = node_id
                    nodes.append({"id": node_id, "kind": "input",
                                  "label": sd, "loom": None})
                edges.append({"source": input_seen[sd], "target": uid(knot),
                              "label": sd})

    return {"nodes": nodes, "edges": edges}


__SYSTEM_NODE__ = "knot-system"

# One strand-dir event notification: a Created/Modified watcher record with
# the recipient knot and the event file path. KnotModified records (knot
# definition changes — the payload is the knot's YAML, no strand_path) do not
# match: the verb is anchored and the struct field is `id:`, not `knot_id:`.
_NOTIFY_RE = re.compile(
    r"\[KNOT\]\[NOTIFY\] (?:Created|Modified) .*"
    r"knot_id: KnotId\(\"([^\"]*)\"\).*"
    r"strand_path: StrandPath\(\"([^\"]*)\"\)")

# A knot status change: `change knot <loom>/<knot>: status <from>→<to>`,
# with the line-leading RFC 3339 timestamp. Any `→processing` is a busy
# start (idle/completed/failed), any `processing→X` a busy end.
_STATE_RE = re.compile(
    r"^\[(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:[+-]\d{2}:\d{2})?)\] "
    r"\[KNOT\]\[STATE\] change knot (?P<ref>[^:]+): "
    r"status (?P<from>[a-z]+)→(?P<to>[a-z]+)")


def parse_strand_events(log_path):
    """Count distinct strand events and total busy minutes per knot.

    A strand event is a file delivered to a knot's strand dir; the service
    log records each notification as a `[KNOT][NOTIFY]` line. A delivery
    that notifies `Created` then `Modified` is one event, so distinct
    (knot, strand_path) pairs are counted, not raw lines.

    Busy minutes sum a knot's closed `X→processing` … `processing→X`
    sessions (timestamps are the line-leading ones). Unclosed sessions —
    a knot still processing at the log tail — are excluded: the figure is
    completed work, not a number that grows while the log grows. A busy
    session cannot span a service restart: `initial snapshot` marks the
    start of each run, and open sessions are dropped there (Knot is a
    single process — a `→processing` in one run was never closed by the
    next run's `processing→X`, and pairing them would bill the whole
    shutdown to the knot). Knots are keyed by bare knot id (the last
    `/`-segment of the `loom/knot` ref in STATE records).
    Returns [{"knot", "count", "minutes"}, ...] — the union of knots with
    events and knots with minutes — sorted by knot name.
    """
    seen = set()
    busy_start = {}    # knot -> datetime
    busy_seconds = {}  # knot -> total closed-session seconds
    with open(log_path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if "[KNOT][STATE] initial snapshot" in line:
                busy_start.clear()  # new service run; sessions don't span it
                continue
            m = _NOTIFY_RE.search(line)
            if m:
                seen.add((m.group(1), m.group(2)))
                continue
            m = _STATE_RE.match(line)
            if m and "processing" in (m["from"], m["to"]):
                try:
                    ts = datetime.datetime.fromisoformat(m["ts"])
                except ValueError:
                    continue
                knot = m["ref"].rsplit("/", 1)[-1]
                if m["to"] == "processing":
                    if knot not in busy_start:  # one session per knot
                        busy_start[knot] = ts
                else:  # processing -> X (completed or failed; both count)
                    start = busy_start.pop(knot, None)
                    if start is not None:
                        busy_seconds[knot] = (
                            busy_seconds.get(knot, 0.0)
                            + (ts - start).total_seconds())
    counts = {}
    for knot, _path in seen:
        counts[knot] = counts.get(knot, 0) + 1
    return [{"knot": k,
             "count": counts.get(k, 0),
             "minutes": round(busy_seconds.get(k, 0.0) / 60.0, 1)}
            for k in sorted(set(counts) | set(busy_seconds))]


def build_system_node(graph):
    """Add the synthetic system node and any unresolved-target nodes."""
    present = {n["id"] for n in graph["nodes"]}
    for e in graph["edges"]:
        if e["source"] == __SYSTEM_NODE__ and __SYSTEM_NODE__ not in present:
            graph["nodes"].append({"id": __SYSTEM_NODE__, "kind": "system",
                                   "label": "Knot (system)", "loom": None})
            present.add(__SYSTEM_NODE__)
        elif e.get("unresolved") and e["source"] not in present:
            graph["nodes"].append({"id": e["source"], "kind": "unresolved",
                                   "label": e["source"], "loom": None})
            present.add(e["source"])
    return graph


def render_html(graph):
    script_dir = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(script_dir, TEMPLATE_NAME), encoding="utf-8") as f:
        template = f.read()
    for marker in (GRAPH_PLACEHOLDER, D3_PLACEHOLDER):
        if marker not in template:
            raise SystemExit("template missing %s placeholder" % marker)
    with open(os.path.join(script_dir, D3_VENDOR_NAME), encoding="utf-8") as f:
        d3_src = f.read()
    data_script = "const data = %s;" % json.dumps(graph, indent=2)
    return (template.replace(D3_PLACEHOLDER, d3_src)
                   .replace(GRAPH_PLACEHOLDER, data_script))


def main(argv=None):
    parser = argparse.ArgumentParser(description="Rig graph extractor")
    parser.add_argument("--out", help="output HTML path (required unless --json)")
    parser.add_argument("--rig", default="rig",
                        help="rig directory, relative to CWD (default: rig)")
    parser.add_argument("--no-state", action="store_true",
                        help="omit the runtime state overlay")
    parser.add_argument("--json", action="store_true",
                        help="print graph JSON to stdout and skip HTML generation")
    parser.add_argument("--log", metavar="PATH",
                        help="knot service log to count strand events from "
                             "(the [KNOT][NOTIFY] records); omit for no "
                             "strand-events legend")
    args = parser.parse_args(argv)

    if not args.json and not args.out:
        parser.error("--out is required unless --json is given")

    if not os.path.isdir(args.rig):
        print("error: rig directory not found: %s" % args.rig, file=sys.stderr)
        return 2

    if args.log and not os.path.isfile(args.log):
        print("error: log file not found: %s" % args.log, file=sys.stderr)
        return 2

    looms = scan_rig(args.rig)
    if not looms:
        print("error: no looms found under %s (expected directories ending in -loom)"
              % args.rig, file=sys.stderr)
        return 2

    overlay = {} if args.no_state else load_state_overlay(os.getcwd(), args.rig)
    graph = build_system_node(build_graph(looms, overlay))
    graph["strand_events"] = (parse_strand_events(args.log) if args.log else [])

    if args.json:
        print(json.dumps(graph, indent=2))
    else:
        html = render_html(graph)
        parent = os.path.dirname(os.path.abspath(args.out))
        os.makedirs(parent, exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(html)
        total_min = round(sum(e["minutes"] for e in graph["strand_events"]), 1)
        print("wrote %s (%d nodes, %d edges, %d strand events, %s min processing)"
              % (args.out, len(graph["nodes"]), len(graph["edges"]),
                 sum(e["count"] for e in graph["strand_events"]), total_min))
    return 0


if __name__ == "__main__":
    sys.exit(main())
