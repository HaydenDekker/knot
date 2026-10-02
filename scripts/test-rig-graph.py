#!/usr/bin/env python3
"""Tests for scripts/rig-graph.py (stdlib only, run with unittest).

python3 scripts/test-rig-graph.py
"""

import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "rig-graph.py")
FIXTURES = os.path.join(os.path.dirname(HERE), "tests", "fixtures", "rig-graph")


def run_graph(cwd, *extra):
    """Run the script with --json in `cwd`; return (rc, parsed_json_or_None, stderr)."""
    proc = subprocess.run(
        [sys.executable, SCRIPT, "--json", *extra],
        cwd=cwd, capture_output=True, text=True,
    )
    data = None
    if proc.returncode == 0:
        data = json.loads(proc.stdout)
    return proc.returncode, data, proc.stderr


def nodes_by_id(graph):
    return {n["id"]: n for n in graph["nodes"]}


def edges(graph):
    return graph["edges"]


def find_edges(graph, **match):
    return [e for e in edges(graph)
            if all(e.get(k) == v for k, v in match.items())]


class FullRigTest(unittest.TestCase):
    """Fixture rig with all five strand-dir forms + state.json overlay."""

    @classmethod
    def setUpClass(cls):
        cls.rc, cls.graph, cls.err = run_graph(os.path.join(FIXTURES, "full"))
        if cls.rc != 0:
            raise unittest.SkipTest("script failed: %s" % cls.err)

    def test_exit_code(self):
        self.assertEqual(self.rc, 0)

    def test_knot_nodes(self):
        nodes = nodes_by_id(self.graph)
        for knot_id, loom in [("planner", "planning-loom"), ("scout", "planning-loom"),
                              ("coder", "build-loom"), ("reviewer", "build-loom"),
                              ("watchdog", "build-loom"), ("archivist", "build-loom")]:
            self.assertIn(knot_id, nodes, "missing knot node %s" % knot_id)
            self.assertEqual(nodes[knot_id]["kind"], "knot")
            self.assertEqual(nodes[knot_id]["loom"], loom)

    def test_input_node(self):
        nodes = nodes_by_id(self.graph)
        self.assertIn("input:project/briefs", nodes)
        self.assertEqual(nodes["input:project/briefs"]["kind"], "input")

    def test_system_node(self):
        nodes = nodes_by_id(self.graph)
        self.assertEqual(nodes["knot-system"]["kind"], "system")

    def test_unresolved_node(self):
        nodes = nodes_by_id(self.graph)
        self.assertEqual(nodes["ghost-knot"]["kind"], "unresolved")

    def test_plain_path_edge(self):
        self.assertEqual(
            find_edges(self.graph, source="input:project/briefs",
                       target="planner", label="project/briefs"),
            [{"source": "input:project/briefs", "target": "planner",
              "label": "project/briefs"}],
        )

    def test_knot_level_edge(self):
        self.assertEqual(
            find_edges(self.graph, source="planner", target="coder",
                       label="PlanCreated"),
            [{"source": "planner", "target": "coder", "label": "PlanCreated"}],
        )  # exact match: no "scope" key on knot-level edges

    def test_loom_level_edges(self):
        # event:planning-loom:PlanCreated fans out to every knot in the loom.
        got = find_edges(self.graph, target="reviewer", label="PlanCreated")
        self.assertEqual({e["source"] for e in got}, {"planner", "scout"})
        # The fan-out is a loom-wide subscription, not knot-specific sends.
        self.assertTrue(all(e.get("scope") == "loom" for e in got))

    def test_wildcard_edges(self):
        # event:*:RunDone fans out to every knot in the rig.
        got = find_edges(self.graph, target="archivist", label="RunDone")
        self.assertEqual({e["source"] for e in got},
                         {"planner", "scout", "coder", "reviewer",
                          "watchdog", "archivist"})
        self.assertTrue(all(e.get("scope") == "all" for e in got))

    def test_rig_level_system_edge(self):
        self.assertEqual(
            find_edges(self.graph, source="knot-system", target="watchdog",
                       label="KnotFailed"),
            [{"source": "knot-system", "target": "watchdog", "label": "KnotFailed"}],
        )

    def test_unresolved_edge_flagged(self):
        got = find_edges(self.graph, source="ghost-knot", target="scout",
                         label="GhostEvent")
        self.assertEqual(len(got), 1)
        self.assertTrue(got[0]["unresolved"])

    def test_edge_count(self):
        # 1 (plain) + 1 (knot-level) + 2 (loom-level) + 6 (wildcard)
        # + 1 (system) + 1 (unresolved) = 12
        self.assertEqual(len(edges(self.graph)), 12)

    def test_state_overlay_applied(self):
        nodes = nodes_by_id(self.graph)
        self.assertEqual(nodes["planner"]["status"], "idle")
        self.assertEqual(nodes["planner"]["last_event_at"],
                         "2026-10-02T10:00:00+10:00")
        self.assertEqual(nodes["coder"]["status"], "processing")

    def test_no_state_flag(self):
        rc, graph, _ = run_graph(os.path.join(FIXTURES, "full"), "--no-state")
        self.assertEqual(rc, 0)
        nodes = nodes_by_id(graph)
        self.assertNotIn("status", nodes["planner"])


class PlainRigTest(unittest.TestCase):
    """Fixture rig with no state.json: overlay absent, lone knot unsubscribed."""

    @classmethod
    def setUpClass(cls):
        cls.rc, cls.graph, cls.err = run_graph(os.path.join(FIXTURES, "plain"))
        if cls.rc != 0:
            raise unittest.SkipTest("script failed: %s" % cls.err)

    def test_exit_code(self):
        self.assertEqual(self.rc, 0)

    def test_no_state_overlay_without_state_json(self):
        for node in self.graph["nodes"]:
            self.assertNotIn("status", node,
                             "unexpected state overlay on %s" % node["id"])

    def test_no_system_or_unresolved_nodes(self):
        kinds = {n["id"]: n["kind"] for n in self.graph["nodes"]}
        self.assertNotIn("knot-system", kinds)
        self.assertFalse(any(k == "unresolved" for k in kinds.values()))

    def test_lone_knot_has_no_event_subscribers(self):
        # Nothing subscribes to `lone`: its only incoming edge is its own
        # input edge; no event edge targets it.
        incoming = [e for e in edges(self.graph) if e["target"] == "lone"]
        self.assertEqual(
            incoming,
            [{"source": "input:project/inbox", "target": "lone",
              "label": "project/inbox"}],
        )

    def test_knot_level_edge(self):
        self.assertEqual(
            find_edges(self.graph, source="producer", target="consumer",
                       label="Done"),
            [{"source": "producer", "target": "consumer", "label": "Done"}],
        )


class ErrorCaseTest(unittest.TestCase):
    def test_missing_rig_dir(self):
        with tempfile.TemporaryDirectory() as tmp:
            rc, data, err = run_graph(tmp, "--rig", "does-not-exist")
        self.assertEqual(rc, 2)
        self.assertIsNone(data)
        self.assertIn("not found", err)

    def test_rig_with_no_looms(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.makedirs(os.path.join(tmp, "rig", "notes"))
            rc, data, err = run_graph(tmp)
        self.assertEqual(rc, 2)
        self.assertIsNone(data)
        self.assertIn("no looms", err)


class DupIdRigTest(unittest.TestCase):
    """Knot ids duplicated across looms get unique (loom-prefixed) node ids,
    and a bare knot-id subscription fans out to every knot with that id."""

    @classmethod
    def setUpClass(cls):
        cls.rc, cls.graph, cls.err = run_graph(os.path.join(FIXTURES, "dup"))
        if cls.rc != 0:
            raise unittest.SkipTest("script failed: %s" % cls.err)

    def test_exit_code(self):
        self.assertEqual(self.rc, 0)

    def test_node_ids_unique(self):
        ids = [n["id"] for n in self.graph["nodes"]]
        self.assertEqual(len(ids), len(set(ids)), "duplicate node ids: %s" % ids)

    def test_loom_prefixed_ids_with_labels_preserved(self):
        nodes = nodes_by_id(self.graph)
        for nid, label in [("alpha-loom:knot-x", "knot-x"), ("beta-loom:knot-x", "knot-x")]:
            self.assertIn(nid, nodes)
            self.assertEqual(nodes[nid]["label"], label)

    def test_knot_level_subscription_fans_out_to_both_looms(self):
        got = {e["source"] for e in
               find_edges(self.graph, target="fan", label="Tick")}
        self.assertEqual(got, {"alpha-loom:knot-x", "beta-loom:knot-x"})


class OutArgTest(unittest.TestCase):
    def test_out_required_without_json(self):
        proc = subprocess.run([sys.executable, SCRIPT],
                              capture_output=True, text=True)
        self.assertNotEqual(proc.returncode, 0)


class HtmlGenerationTest(unittest.TestCase):
    """--out generation against the plain fixture rig (no state overlay)."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="rig-graph-html-")
        cls.out = os.path.join(cls.tmp, "nested", "dir", "graph.html")
        proc = subprocess.run(
            [sys.executable, SCRIPT, "--out", cls.out, "--rig", "rig"],
            cwd=os.path.join(FIXTURES, "plain"),
            capture_output=True, text=True,
        )
        cls.rc, cls.stderr = proc.returncode, proc.stderr
        if cls.rc == 0 and os.path.isfile(cls.out):
            with open(cls.out, encoding="utf-8") as f:
                cls.html = f.read()

    def test_exit_code(self):
        self.assertEqual(self.rc, 0, self.stderr)

    def test_output_written_with_parent_dir_creation(self):
        self.assertTrue(os.path.isfile(self.out))

    def test_placeholders_substituted(self):
        self.assertNotIn("/*__GRAPH_JSON__*/", self.html)
        self.assertNotIn("/*__D3__*/", self.html)

    def test_graph_json_embedded_exactly_once(self):
        self.assertEqual(self.html.count("const data = "), 1)

    def test_embedded_json_round_trips(self):
        m = re.search(r"const data = (\{.*?\});\nconst svg", self.html, re.DOTALL)
        self.assertIsNotNone(m, "embedded data script not found")
        data = json.loads(m.group(1))
        self.assertEqual(len(data["nodes"]), 5)   # 3 knots + 2 inputs
        self.assertEqual(len(data["edges"]), 3)

    def test_d3_inlined(self):
        self.assertIn("d3js.org", self.html)
        self.assertIn("forceSimulation", self.html)

    def test_interaction_wiring(self):
        # Phase 4 interactions must be present in the generated HTML.
        # Legend: checkboxes (filter) + loom rows (highlight on click).
        # (The checkboxes are D3-generated at runtime — .attr(...), not markup.)
        self.assertIn('.attr("type", "checkbox")', self.html)
        self.assertIn("loom-row", self.html)
        # Highlighting: node click, edge hit-lines, background clear.
        # Selections matched by identity (D3 v7 listeners get (event, datum)).
        self.assertIn("activeEdge", self.html)
        self.assertNotIn("activeEdgeIndex", self.html)
        self.assertIn("activeNode", self.html)
        self.assertIn("activeLoom", self.html)
        self.assertIn("function highlight()", self.html)
        self.assertIn("function visibility()", self.html)
        self.assertIn("function clearSelection()", self.html)
        self.assertIn('svg.on("click.clear", clearSelection)', self.html)
        self.assertIn('"pointer-events", "stroke"', self.html)  # edge hit lines
        self.assertIn("ev.stopPropagation()", self.html)
        # Legend rows are built with a real enter() selection — one row per
        # loom (a single append would bind only the first datum).
        self.assertIn(".data(looms).enter()", self.html)
        # Loom highlight includes the nodes feeding into the loom.
        self.assertIn("feeding node", self.html)
        # Edge labels: hidden by default, lit on highlight / hover;
        # wide-scope (loom/wildcard) fan-out edges are dotted and suffixed.
        self.assertIn(".edge-label.lit, .edge-label.hover", self.html)
        self.assertIn('linkHit', self.html)
        self.assertIn('"mouseover"', self.html)
        self.assertIn("scoped-edge", self.html)
        self.assertIn('d.scope ? " · " + d.scope', self.html)
        # Filtering state: hidden looms hide knots and touching edges.
        self.assertIn("hiddenLooms", self.html)
        self.assertIn("knotVisible", self.html)
        self.assertIn("edgeVisible", self.html)

    def test_no_external_network_references(self):
        # Offline guarantee: no src/href/link/@import/url() pointing at a
        # network origin. The only literal URLs allowed are non-fetched
        # namespace identifiers inside the vendored D3 (W3C DOM namespaces)
        # and its copyright comment.
        for pattern in (r'src="[\'\"]?https?://', r'href="[\'\"]?https?://',
                        r"<link[ >]", r"@import\s+url", r"url\(\s*https?://"):
            self.assertIsNone(re.search(pattern, self.html),
                              "external reference pattern found: %s" % pattern)
        allowed = ("http://www.w3.org/", "https://d3js.org")
        for url in re.findall(r"https?://[^\'\"\) ]*", self.html):
            self.assertTrue(url.startswith(allowed),
                            "unexpected network URL: %s" % url)


if __name__ == "__main__":
    unittest.main()
