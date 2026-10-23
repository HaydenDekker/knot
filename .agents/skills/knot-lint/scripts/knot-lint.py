#!/usr/bin/env python3
"""knot-lint — static rig consistency audit (advisory).

Runs the five check families of the behaviour-in-skill rule over a rig
directory and emits an advisory report. Report only: it never modifies
anything in the audited rig; rectification is a separate, user-walked act.

Check families:
  1. Inline behaviour  — knot/profile prose beyond the measurable boundary
  2. Shared documents  — project docs referenced by >=2 rig docs;
                         reader vs editor split (>=2 editors is a finding)
  3. Event pairing     — emitted events without a strand-dir consumer;
                         event subscriptions with no named producer
  4. Format-bootstrap  — format pointers whose source of truth is a
                         runtime data file or the file they govern
  5. Naming leg        — the shared-document table classified
                         state/log/doc (skill-data-filenames /
                         project-lint-files defect family)

Stdlib only. Does not require the Knot service to be running.

Usage:
  python3 knot-lint.py RIG_DIR [--project-dir DIR] [--report PATH] [--json]
  python3 knot-lint.py --selftest [FIXTURES_DIR]

Exit codes: 0 = no FAIL findings, 1 = FAILs present, 2 = usage error.
"""

import argparse
import datetime
import json
import os
import re
import sys

# --- the measurable boundary (knot-design v1.7.0 / docs/design-guide.md) ---
# Prose = body excluding frontmatter, fenced code blocks, and table lines.
# Calibrated 2026-10-23 (plan 098 phase 1): the knot repo's rig (wiring
# knots, 2 words) sits far below; the software-factory rig's rig
# documents floor at 425 words. The limits sit in the gap.
THRESHOLDS = {
    "knot_prose_words": 80,
    "knot_numbered_steps": 3,
    "profile_prose_words": 400,
    "profile_numbered_steps": 5,
}

WRITER_VERB_RX = re.compile(
    r"\b(?:writes?|written|writing|appends?|appended|updates?|updated"
    r"|records?|creates?|created|adds?|added|maintains?|maintaining"
    r"|owns?|manages?|managing|edits?|edited|logs?|logged|journals?"
    r"|tracks?|tracked|assigns?|assigned)\b")
FORMAT_WORDS = ["format", "shape", "template", "schema", "entry", "row"]
SOURCE_WORDS = ["declared", "defined", "documented", "stated", "lives", "see",
                "spelled", "carried"]

# Naming classifier (skill-data-filenames conventions + project-lint-files
# class defaults). Log patterns run first — the state/log boundary is the
# load-bearing one.
LOG_PATTERNS = [r"^-?log", r"history", r"archive", r"trail", r"records?/",
                r"register", r"discoveries", r"findings", r"issues/"]
STATE_PATTERNS = [r"state", r"master", r"index", r"ledger", r"scope", r"watch",
                  r"triage", r"matrix", r"progress-table", r"inventory",
                  r"vcrm"]


def parse_frontmatter(text):
    """Flat `key: value` YAML frontmatter (event URIs may contain colons)."""
    m = re.match(r"\A---\s*\n(.*?)\n---\s*(\n|\Z)", text, re.DOTALL)
    if not m:
        return {}, text
    fields = {}
    for line in m.group(1).splitlines():
        if not line.strip() or line.lstrip().startswith("#") or ":" not in line:
            continue
        key, _, value = line.partition(":")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        fields[key.strip()] = value
    return fields, text[m.end():]


def body_prose(body):
    """Prose lines: no frontmatter (already split), no code fences, no tables."""
    body = re.sub(r"```.*?```", "", body, flags=re.S)
    return [l for l in body.splitlines()
            if l.strip() and not l.strip().startswith("|")]


def count_words(lines):
    return len(" ".join(lines).split())


def count_numbered_steps(lines):
    """Distinct numbered steps (1. 2. 3. …) in prose lines."""
    seen = set()
    for line in lines:
        m = re.match(r"\s*(\d+)\.\s", line)
        if m:
            seen.add(int(m.group(1)))
    return len(seen)


def count_procedural_headings(body):
    return len(re.findall(
        r"^#{1,4}\s+(?:Your\s+)?(?:Task|Procedure|Steps|Process|Contract|How\s+to)\b",
        body, flags=re.M | re.I))


def scan_rig(rig_dir):
    """Walk the rig: looms (dirs ending -loom), knots, profiles."""
    looms = {}
    for entry in sorted(os.listdir(rig_dir)):
        path = os.path.join(rig_dir, entry)
        if not (os.path.isdir(path) and entry.endswith("-loom")
                and not entry.startswith(".")):
            continue
        knots = []
        for name in sorted(os.listdir(path)):
            if not (name.endswith(".md") and not name.startswith(".")):
                continue
            with open(os.path.join(path, name), encoding="utf-8") as f:
                text = f.read()
            fields, body = parse_frontmatter(text)
            knots.append({
                "id": fields.get("name") or os.path.splitext(name)[0],
                "loom": entry,
                "rel": os.path.join(entry, name),
                "fields": fields,
                "body": body,
            })
        if knots:
            looms[entry] = knots
    profiles = []
    pdir = os.path.join(rig_dir, "profiles")
    if os.path.isdir(pdir):
        for name in sorted(os.listdir(pdir)):
            if not (name.endswith(".md") and not name.startswith(".")):
                continue
            with open(os.path.join(pdir, name), encoding="utf-8") as f:
                text = f.read()
            fields, body = parse_frontmatter(text)
            profiles.append({
                "id": fields.get("name") or os.path.splitext(name)[0],
                "rel": os.path.join("profiles", name),
                "fields": fields,
                "body": body,
            })
    return looms, profiles


DOC_REF = re.compile(r"project/[A-Za-z0-9_\-./]*?[A-Za-z0-9_\-]\.md")
# Static emission evidence. Event names are CamelCase — the captures are
# case-sensitive; `event: None` is the "emit nothing" sentinel and is
# filtered at collection time.
EMIT_EVIDENCE = [
    re.compile(r"(?i:\bemit(?:s|ting|ted)?\b)[^\n`]{0,40}`([A-Z][A-Za-z0-9_\-]+)`"),
    re.compile(r"(?i:\bemit(?:s|ting|ted)?\b)\s+event\s+([A-Z][A-Za-z0-9_\-]+)"),
    re.compile(r"\bevent:\s*([A-Z][A-Za-z0-9_\-]+)"),
]
EVENT_URI = re.compile(r"^event:([^:]+):([A-Za-z0-9_\-]+)$")


def classify_reference(ref_line):
    """reader vs editor from the referrer's line wording."""
    return "editor" if WRITER_VERB_RX.search(ref_line.lower()) else "reader"


def classify_name(path):
    """state / log / doc, from the path (skill-data-filenames + lint-files)."""
    base = os.path.basename(path)
    for pat in LOG_PATTERNS:
        if re.search(pat, base) or re.search(pat, path):
            return "log"
    for pat in STATE_PATTERNS:
        if re.search(pat, base) or re.search(pat, path):
            return "state"
    return "doc"


def audit(rig_dir, project_dir, rig_name):
    looms, profiles = scan_rig(rig_dir)
    rig_docs = (
        [{"kind": "knot", **k} for ks in looms.values() for k in ks]
        + [{"kind": "profile", **p} for p in profiles]
    )
    findings = []

    def add(family, code, severity, subject, detail, rect):
        findings.append({"family": family, "code": code, "severity": severity,
                         "subject": subject, "detail": detail,
                         "rectification": rect})

    knot_ids = {k["id"] for ks in looms.values() for k in ks}
    loom_ids = set(looms)
    # rig id (rig-scoped system events) + the engine's rig-event token
    # (RIG_EVENT_ENGINE_TOKEN — `event:knot:<SystemEvent>`)
    known_targets = knot_ids | loom_ids | {rig_name, "knot"}

    # --- family 1: inline behaviour -------------------------------
    family1 = []
    for d in rig_docs:
        kind = d["kind"]
        prose = body_prose(d["body"])
        words = count_words(prose)
        steps = count_numbered_steps(prose)
        heads = count_procedural_headings(d["body"])
        if kind == "knot":
            wl, sl = THRESHOLDS["knot_prose_words"], THRESHOLDS["knot_numbered_steps"]
        else:
            wl, sl = THRESHOLDS["profile_prose_words"], THRESHOLDS["profile_numbered_steps"]
        marks = []
        if words > wl:
            marks.append(f"prose {words} words (> {wl})")
        if steps >= sl:
            marks.append(f"{steps} numbered steps (>= {sl})")
        if heads >= 2:
            marks.append(f"{heads} procedural headings (>= 2)")
        if marks:
            family1.append((d["rel"], kind, words, steps, heads, "; ".join(marks)))
            add(1, "K1", "WARN", d["rel"],
                f"{kind} exceeds the wiring boundary: " + "; ".join(marks),
                "Move the procedure to a skill referenced by the profile or the "
                "intent line; keep identity, wiring, and the one-line intent.")

    # --- shared-document table (families 2 + 5 share it) ----------
    refs = {}  # doc -> {referrer_rel: (line, role)}
    for d in rig_docs:
        for line in d["body"].splitlines():
            for doc in sorted(set(DOC_REF.findall(line))):
                role = classify_reference(line)
                cur = refs.setdefault(doc, {}).setdefault(d["rel"], (line.strip()[:120], role))
                if role == "editor" and cur[1] != "editor":
                    refs[doc][d["rel"]] = (cur[0], "editor")  # editor dominates
    shared = {doc: m for doc, m in refs.items() if len(m) >= 2}

    family2 = []
    for doc in sorted(shared):
        editors = sorted(r for r, (_, role) in shared[doc].items() if role == "editor")
        family2.append((doc, sorted(shared[doc]), editors))
        if len(editors) >= 2:
            add(2, "K2", "FAIL", doc,
                f"{len(editors)} rig documents hold write rights "
                f"({', '.join(editors)}); referencers: {len(shared[doc])}",
                "Designate one editor; reduce the others to reads "
                "(single-editor norm).")

    # --- family 3: event pairing ----------------------------------
    consumers = []  # (rel, target, event)
    for d in rig_docs:
        if d["kind"] != "knot":
            continue
        for sd in re.findall(r"strand-dir:\s*\"?([^\n\"]+)", d["body"] + "\n" + "\n".join(
                f"{k}: {v}" for k, v in d["fields"].items())):
            m = EVENT_URI.match(sd.strip())
            if m:
                consumers.append((d["rel"], m.group(1), m.group(2)))
                if m.group(1) not in known_targets:
                    add(3, "K3B", "WARN", d["rel"],
                        f"subscription event:{m.group(1)}:{m.group(2)} — "
                        f"no knot or loom named `{m.group(1)}` in the rig",
                        "Point the subscription at an existing knot/loom "
                        "(or the rig id for system events), or delete it.")
    emitted = {}  # event -> [(rel, sample)]
    for d in rig_docs:
        for rx in EMIT_EVIDENCE:
            for m in rx.findall(d["body"]):
                if m == "None":
                    continue  # the "emit nothing" sentinel
                emitted.setdefault(m, []).append((d["rel"], m))
    subs = {(t, e) for _, t, e in consumers}
    for ev in sorted(emitted):
        has_consumer = any(e == ev for _, e in subs)
        if not has_consumer:
            rels = sorted({r for r, _ in emitted[ev]})
            add(3, "K3A", "WARN", ev,
                f"emission evidence in {', '.join(rels)}; no "
                f"`strand-dir: event:*:{ev}` consumer in the rig",
                "Add the consumer subscription (or remove the emission — "
                "an event nobody consumes is dead weight).")

    # --- family 4: format-bootstrap circularity --------------------
    family4 = []
    for d in rig_docs:
        for i, line in enumerate(d["body"].splitlines()):
            lowered = line.lower()
            if not any(w in lowered for w in FORMAT_WORDS):
                continue
            if not any(w in lowered for w in SOURCE_WORDS):
                continue
            for doc in DOC_REF.findall(line):
                target_class = classify_name(doc)
                self_ref = d["rel"].endswith(os.path.basename(doc)) and doc not in ("",)
                if target_class != "doc" or self_ref:
                    why = (f"source of truth is the file itself" if self_ref
                           else f"source of truth is a {target_class}-class runtime file ({doc})")
                    family4.append((d["rel"], doc, line.strip()[:120]))
                    add(4, "K4", "FAIL", d["rel"],
                        f"line {i + 1} points at `{doc}` for a format — {why}",
                        "Move the format definition to a skill or normative "
                        "document; point the rig document at it.")
                    break  # one finding per line

    # --- family 5: naming leg --------------------------------------
    family5 = []
    for doc in sorted(shared):
        cls = classify_name(doc)
        editors = [r for r, (_, role) in shared[doc].items() if role == "editor"]
        format_targets = any(doc == t for _, t, _ in family4)
        family5.append((doc, cls, len(shared[doc]), len(editors)))
        if cls == "log" and (editors or format_targets or len(shared[doc]) >= 3):
            add(5, "K5", "WARN", doc,
                f"log-class file referenced by {len(shared[doc])} rig documents"
                + (f", {len(editors)} with write rights" if editors else "")
                + ("; also a format source" if format_targets else ""),
                "Classify and name it for its dominant contract "
                "(skill-data-filenames); split state from history if hybrid.")

    return {
        "rig": rig_name,
        "rig_dir": os.path.abspath(rig_dir),
        "project_dir": os.path.abspath(project_dir) if project_dir else None,
        "looms": {l: [k["id"] for k in ks] for l, ks in looms.items()},
        "profiles": [p["id"] for p in profiles],
        "findings": findings,
        "family1": [dict(file=r, kind=k, words=w, steps=s, headings=h, marks=m)
                    for r, k, w, s, h, m in family1],
        "family2": [dict(doc=d, referencers=r, editors=e) for d, r, e in family2],
        "family3": [dict(rel=r, target=t, event=e) for r, t, e in consumers]
                   + [dict(event=ev, evidence=sorted({x for x, _ in v}))
                      for ev, v in sorted(emitted.items())],
        "family4": [dict(file=r, doc=t, line=l) for r, t, l in family4],
        "family5": [dict(doc=d, cls=c, referrers=n, editors=e)
                    for d, c, n, e in family5],
    }


def render(result, thresholds):
    now = datetime.date.today().isoformat()
    f = result["findings"]
    lines = [
        "# Knot Lint Report (advisory)",
        "",
        f"**Run:** {now} · **Rig:** `{result['rig']}` · "
        f"**Rig dir:** `{result['rig_dir']}` · "
        f"**Project dir:** `{result['project_dir'] or '—'}`",
        f"**Thresholds:** knot prose > {thresholds['knot_prose_words']} w / "
        f">= {thresholds['knot_numbered_steps']} steps; profile prose > "
        f"{thresholds['profile_prose_words']} w / "
        f">= {thresholds['profile_numbered_steps']} steps (prose excludes "
        "frontmatter, code fences, and table lines)",
        "",
        "Advisory only — this report changes nothing in the rig. One primary "
        "rectification per finding; rectification is a separate, user-walked act.",
        "",
        "## 1. Rig inventory",
        "",
    ]
    for loom, knots in result["looms"].items():
        lines.append(f"- `{loom}/` — {', '.join('`%s`' % k for k in knots)}")
    if result["profiles"]:
        lines.append("- `profiles/` — " + ", ".join(f"`{p}`" for p in result["profiles"]))
    lines += ["", "## 2. Findings", ""]
    if not f:
        lines.append("No findings.")
    else:
        lines += ["| Sev | Family | Code | Subject | Detail | Rectification (primary) |",
                  "|-----|--------|------|---------|--------|-------------------------|"]
        order = {"FAIL": 0, "WARN": 1}
        for x in sorted(f, key=lambda x: (order[x["severity"]], x["family"], x["code"], x["subject"])):
            lines.append(f"| **{x['severity']}** | {x['family']} | {x['code']} "
                         f"| `{x['subject']}` | {x['detail']} | {x['rectification']} |")
    lines += ["", "## 3. Check 1 — inline behaviour (boundary crossings)", ""]
    if result["family1"]:
        lines += ["| File | Kind | Prose (w) | Steps | Headings | Marks |",
                  "|------|------|----------:|------:|---------:|-------|"]
        for x in result["family1"]:
            lines.append(f"| `{x['file']}` | {x['kind']} | {x['words']} | {x['steps']} "
                         f"| {x['headings']} | {x['marks']} |")
    else:
        lines.append("No rig document exceeds the wiring boundary.")
    lines += ["", "## 4. Check 2 — shared-document table (project docs referenced by >= 2 rig docs)", ""]
    if result["family2"]:
        lines += ["| Doc | Referrers | Readers | Editors | Verdict |",
                  "|-----|----------:|--------:|--------:|---------|"]
        for x in result["family2"]:
            readers = len(x["referencers"]) - len(x["editors"])
            verdict = "**FAIL — shared editors**" if len(x["editors"]) >= 2 else "ok (single editor)" if len(x["editors"]) <= 1 else "—"
            lines.append(f"| `{x['doc']}` | {len(x['referencers'])} | {readers} "
                         f"| {len(x['editors'])} | {verdict} |")
        for x in result["family2"]:
            if x["editors"]:
                lines.append(f"\n- `{x['doc']}` editors: " +
                             ", ".join(f"`{e}`" for e in x["editors"]))
    else:
        lines.append("No project document is referenced by >= 2 rig documents.")
    lines += ["", "## 5. Check 3 — event pairing", ""]
    lines.append("Subscriptions:")
    if result["family3"]:
        for x in result["family3"]:
            if "target" in x:
                lines.append(f"- `{x['rel']}` listens for `event:{x['target']}:{x['event']}`")
    else:
        lines.append("- none")
    lines.append("")
    lines.append("Emission evidence (no consumer = finding K3A):")
    em = [x for x in result["family3"] if "evidence" in x]
    if em:
        for x in em:
            lines.append(f"- `{x['event']}` — evidence in " +
                         ", ".join(f"`{e}`" for e in x["evidence"]))
    else:
        lines.append("- none")
    lines += ["", "## 6. Check 4 — format-bootstrap circularity", ""]
    if result["family4"]:
        for x in result["family4"]:
            lines.append(f"- `{x['file']}` points at `{x['doc']}`: “{x['line']}”")
    else:
        lines.append("No format points at its own governed file or a runtime data file.")
    lines += ["", "## 7. Check 5 — naming leg (shared table classified)", ""]
    if result["family5"]:
        lines += ["| Doc | Class | Referrers | Editors | Note |",
                  "|-----|-------|----------:|--------:|------|"]
        for x in result["family5"]:
            note = "**log class under rig write pressure**" if x["cls"] == "log" and (x["editors"] or x["referrers"] >= 3) else "ok"
            lines.append(f"| `{x['doc']}` | {x['cls']} | {x['referrers']} "
                         f"| {x['editors']} | {note} |")
    else:
        lines.append("Nothing to classify (empty shared table).")
    lines += ["", "## 8. Provenance", "",
              f"- Thresholds from the measurable boundary (knot-design v1.7.0, "
              f"calibrated plan 098 phase 1): {json.dumps(thresholds)}",
              "- Editor/reader split: line-wording heuristic (writer verbs on the "
              "line carrying the reference); one finding per (family, subject)",
              "- Emission evidence: static patterns (emit(s) `Event`, `event: Event`) "
              "— runtime-injected emission is invisible to a static audit",
              "- This report is data, not standard (the project-lint-files convention)."]
    return "\n".join(lines) + "\n"


# --- selftest ----------------------------------------------------------
# The golden finding set for references/fixtures/bad-rig (the good rig's
# golden is the empty set). Selftest runs the audit over the on-disk
# fixture trees and diffs against these.

GOOD_GOLDEN = {"findings": [], "family2": [], "family5": []}
BAD_GOLDEN = {
    "findings": [
        {"family": 1, "code": "K1", "severity": "WARN",
         "subject": "profiles/chatty.md"},
        {"family": 1, "code": "K1", "severity": "WARN",
         "subject": "work-loom/procedural-knot.md"},
        {"family": 2, "code": "K2", "severity": "FAIL",
         "subject": "project/ops/history.md"},
        {"family": 2, "code": "K2", "severity": "FAIL",
         "subject": "project/shared/ledger.md"},
        {"family": 3, "code": "K3A", "severity": "WARN", "subject": "OrphanEvent"},
        {"family": 3, "code": "K3B", "severity": "WARN",
         "subject": "work-loom/ghost-listener.md"},
        {"family": 4, "code": "K4", "severity": "FAIL",
         "subject": "profiles/format-keeper.md"},
        {"family": 5, "code": "K5", "severity": "WARN",
         "subject": "project/ops/history.md"},
    ],
    # family 2 table: ledger (3 referrers, 3 editors), history (2
    # referrers, 2 editors), style.md (2 readers, 0 editors) — all three
    # appear; ledger and history are findings, style is not.
    "family2_docs": ["project/docs/style.md", "project/ops/history.md",
                     "project/shared/ledger.md"],
    "family5_rows": [
        {"doc": "project/docs/style.md", "cls": "doc"},
        {"doc": "project/ops/history.md", "cls": "log"},
        {"doc": "project/shared/ledger.md", "cls": "state"},
    ],
}


def finding_key(x):
    return (x["family"], x["code"], x["severity"], x["subject"])


def selftest(fixtures_dir):
    failures = []
    for name, golden in (("good-rig", GOOD_GOLDEN), ("bad-rig", BAD_GOLDEN)):
        rig = os.path.join(fixtures_dir, name)
        if not os.path.isdir(rig):
            failures.append(f"fixture tree missing: {rig}")
            continue
        project = os.path.join(rig, "project") if os.path.isdir(os.path.join(rig, "project")) else None
        result = audit(rig, project, name)

        keys = sorted(finding_key(x) for x in result["findings"])
        exp = sorted(finding_key(x) for x in golden["findings"])
        if keys != exp:
            failures.append(f"{name} findings mismatch:\n  expected {exp}\n  got      {keys}")
        got_docs = sorted(x["doc"] for x in result["family2"])
        if got_docs != sorted(golden.get("family2_docs", [])):
            failures.append(f"{name} family2 table mismatch: {got_docs}")
        got5 = sorted((x["doc"], x["cls"]) for x in result["family5"])
        exp5 = sorted((x["doc"], x["cls"]) for x in golden.get("family5_rows", []))
        if got5 != exp5:
            failures.append(f"{name} family5 rows mismatch: {got5} != {exp5}")

        # the report renders and carries every finding as its own row
        report = render(result, THRESHOLDS)
        for x in golden["findings"]:
            if x["code"] not in report:
                failures.append(f"{name} report missing finding code {x['code']}")
        if not golden["findings"] and "No findings." not in report:
            failures.append(f"{name} report missing 'No findings.'")

    if failures:
        print("SELFTEST FAIL")
        for x in failures:
            print(" -", x)
        return 1
    print("selftest: OK (good rig: 0 findings; bad rig: "
          f"{len(BAD_GOLDEN['findings'])} findings, exact match)")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("rig", nargs="?", help="rig directory (default: rig)")
    ap.add_argument("--project-dir", default=None,
                    help="project/ document tree (default: auto — the rig dir "
                         "itself if it holds project/, else the parent's)")
    ap.add_argument("--report", default=None, help="also write the report here")
    ap.add_argument("--json", action="store_true", help="emit JSON to stdout")
    ap.add_argument("--selftest", nargs="?", const="auto", default=None,
                    metavar="FIXTURES_DIR",
                    help="run the fixture selftest (fixtures dir default: "
                         "the skill's references/fixtures)")
    args = ap.parse_args(argv)

    if args.selftest is not None:
        skill_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        fixtures = (args.selftest if args.selftest != "auto"
                    else os.path.join(skill_root, "references", "fixtures"))
        return selftest(fixtures)

    rig_dir = args.rig or "rig"
    if not os.path.isdir(rig_dir):
        print(f"knot-lint: rig dir not found: {rig_dir}", file=sys.stderr)
        return 2
    project_dir = args.project_dir
    if project_dir is None:
        if os.path.isdir(os.path.join(rig_dir, "project")):
            project_dir = os.path.join(rig_dir, "project")
        else:
            parent = os.path.dirname(os.path.abspath(rig_dir))
            project_dir = os.path.join(parent, "project") \
                if os.path.isdir(os.path.join(parent, "project")) else None
    rig_name = os.path.basename(os.path.abspath(rig_dir))
    result = audit(rig_dir, project_dir, rig_name)
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print(render(result, THRESHOLDS))
    if args.report:
        os.makedirs(os.path.dirname(os.path.abspath(args.report)), exist_ok=True)
        with open(args.report, "w", encoding="utf-8") as f:
            f.write(render(result, THRESHOLDS))
        print(f"wrote {args.report}", file=sys.stderr)
    return 1 if any(x["severity"] == "FAIL" for x in result["findings"]) else 0


if __name__ == "__main__":
    sys.exit(main())
