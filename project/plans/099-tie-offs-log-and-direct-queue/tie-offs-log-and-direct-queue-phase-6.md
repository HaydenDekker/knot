# Phase 6: Skills repointing

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 5 is green and committed** (it is). The code
surface is settled — this phase changes documents only, so the panes are
an audit, not a test suite. Trigger: a live test-rig run after v0.63.0
confused the agent over the new queueing mechanics because the skills
still taught the retired ones.

**The target mechanics the skills must teach** (the post-099 reality):

- `tie-offs/<rig>/` holds exactly: `state.json`, `knot-service.log`,
  `tie-offs.log` (one accumulating tie-off log — the service is the sole
  writer), and `events/` (the transient queue). **No per-knot
  `tie-off-{knot-name}.md` files, no per-`EventId` dispatch
  directories, no dispatched `.md` strand files.**
- Events parsed from a producer's tie-off are **queued directly** — one
  payload-bearing queue entry per subscriber in `events/*.json` (the
  `Event` kind). The consumer runs from the queue entry; the payload is
  in the entry body, never in a file on disk.
- `event:` URIs in knot/loom definitions are **subscription
  descriptors** only — they are not resolved to watched paths.
- Manual event dispatch is a **direct queue write** of an `Event`-kind
  entry into `events/`; `knot step --event <id>` addresses entries by
  id. There is no dispatch directory to drop a file into.
- The durable trace chain: service log (live) → git commit **trailers**
  (`session:`, `event:`, `source:`, `original_strand:`) →
  `tie-offs.log` section (structured header line: `event-id=`, `source=`,
  `session=`, `original-strand=`; slim event-audit blocks, no payload
  bodies). The full narrative is reachable via `pi --session-id`.
- The read norm travels with this phase: **tie-offs are logs — query
  (grep / git log), never whole-file read; `state.json` for
  snapshots; git is the rotation.**

## Checklist
- [x] **knot-dispatch** (`.agents/skills/knot-dispatch/SKILL.md`) —
      the deepest rework: the "manual event trigger" section (drop an
      `event-manual.md` into
      `tie-offs/<rig>/{consumer-loom-id}/{EventId}/`) is replaced by the
      direct queue write (hand-write an `Event`-kind entry with a
      `delivery` object into `tie-offs/<rig>/events/`, `knot
      step --event <id>`); the queue/event section's dispatch-directory
      entries and the "Event queue vs dispatch directories" layout
      collapse to the queue alone; the pipeline narrative ("Event blocks
      parsed → dispatched to consumer knots") becomes "queued directly —
      one payload-bearing entry per subscriber"; tie-off-completion
      verification (per-knot `tie-off-{knot-name}.md`) re-points to the
      `tie-offs.log` section; the frontmatter `description` loses
      "event file creation in dispatch directories"
- [x] **knot-manage** (`.agents/skills/knot-manage/SKILL.md`) — all
      per-knot tie-off paths → `tie-offs/<rig>/tie-offs.log` sections
      (grep by loom/knot/`event-id=`, never whole-file read); the
      producer→consumer interaction trace re-points to `tie-offs.log`
      sections + commit trailers (the `event:` / `source:` / `session:`
      matching, the `pi --session-id` link) instead of "check the
      dispatch directory"; the dead-subscription diagnosis
      ("the dispatch directory is empty") re-points to queue/log
      evidence; the `cat tie-offs/rig/planning-loom/tie-off-prd-planner.md`
      example and the "Event blocks in the producer's tie-off body"
      section reworded (the blocks are in the commit body / the agent's
      response — `tie-offs.log` carries only the slim audit line); the
      runtime-migration note's dispatch-directory paragraph retires
- [x] **knot-inspect** (`.agents/skills/knot-inspect/SKILL.md`) — the
      `last_tie_off_path` example and the service-log example lines
      (`tie-off=...tie-off-goals-review.md`, the `dispatch=` fields)
      re-point to the unified-log path and the post-Phase-3 line grammar
      (the structured trace fields); any event-dispatch-directory
      reference removed
- [x] **knot-analyst** (`.agents/skills/knot-analyst/SKILL.md`) — "Read
      the tie-off files" re-points to querying `tie-offs.log`; the
      event-dead-letter diagnosis ("event dispatch dirs with files but
      consumer idle") re-points to queue-entry evidence (entries that
      persist/are re-loaded under `[startup] loaded N persisted
      event(s)`); the `StrandSkipped` note checked against the
      queue-only reality
- [x] **knot-create** (`.agents/skills/knot-create/SKILL.md`) — the
      runtime-tree diagram (per-knot `tie-off-{knot-name}.md` +
      `{event-id}/` dispatch subdirectory) re-drawn as
      `tie-offs.log` + `events/`; the "Knot will create and watch the
      dispatch directory automatically" notes reworded to the
      subscription-descriptor semantics (an `event:` URI is a
      subscription, not a watched path); the per-knot tie-off path in
      the verification steps re-points
- [x] **knot-design** (`.agents/skills/knot-design/SKILL.md`) — the
      event-mechanics walkthrough ("Knot discovers the subscription and
      creates a dispatch directory … the dispatcher writes the file into
      the consumer's dispatch directory") reworked to the direct queue
      write; the self-loop-break warning ("an event dispatch dir
      re-triggers every knot watching that dir — the dispatcher keys off
      the watching knot") reworked to the queue-entry semantics (one
      entry per subscriber, keyed by the subscription); guidance that
      tells a knot to "read its own tie-off file" re-points to querying
      its own `tie-offs.log` sections (or git)
- [x] **knot-init glossary** (`.agents/skills/knot-init/knot-glossary.md`)
      — the two-artifact event-pipeline section (event blocks vs
      "dispatched event strand files"), the `EventUri` resolution
      ("Knot resolves this to the dispatch subdirectory … and watches
      it"), the runtime-tree diagrams, and the pipeline diagram all
      re-point to the queue-entry model with `event:` URIs as
      subscription descriptors; this file is copied to `~/.agents/` by
      `knot-init`, so it must be deploy-verified too
- [x] **knot-template** (`.agents/skills/knot-template/`) — sweep
      `knot-tie-off.md` and `knot-tie-off-content.md` against the
      current hard-coded contract in `src/domain/events.rs`: the
      `occurred: false` wording ("not dispatched"), and anything that
      implies the agent reads or writes tie-off files; note the
      tie-off's durable homes (the commit body; `tie-offs.log` audit
      line) where the templates speak about where content ends up
- [x] **knot-abstractions / knot-start / knot (master)** — verify-and-fix
      pass: `knot-abstractions`' runtime-tree description and
      dispatch-directory mention; `knot-start`'s queue recovery
      narrative (queue entries now carry payloads — the "interrupted
      strand's event file survives" wording); the master's routing-table
      descriptions
- [x] **Audit** — repo-wide grep of `.agents/skills/` (and
      `AGENTS.md`) for the retired tokens comes back clean (or each hit
      is an explicit retirement note): `tie-off-{knot-name}.md` paths,
      `tie-offs/<rig>/{loom}/{event-id}/` dispatch paths, "dispatched
      strand file", "dispatch directory" (outside retirement notes),
      `dispatch=` log fields — and the read norm is present in the
      read-side skills (knot-manage, knot-inspect, knot-analyst)
- [x] **Deploy** — copy the updated skills to the production locations
      per `AGENTS.md` (sub-skills → `~/.agents/skills-library/`, the
      master + glossary → `~/.agents/skills/`) and run the diff
      verification loop for every copied file (incl. `knot-glossary.md`
      and the `knot-template` companion files)
- [ ] **Field test** — re-run the test project that got confused: the
      agent navigates the new mechanics from the skills alone (manual
      event, `knot step`, tie-off inspection) without drifting back to
      dispatch directories or per-knot tie-off paths

## Deviations
- **knot-template: no file changes.** `knot-tie-off.md` is the source of
  the binary's hard-coded default (`include_str!` in
  `src/domain/events.rs`) — its "not dispatched" wording mirrors the
  binary's own doc comments and "never write or read directly from
  `tie-offs/` files" is *more* true post-099 (the agent must not touch
  `tie-offs.log` or `events/` either). `knot-tie-off-content.md`
  carries no path or mechanism claims. Both verified accurate as-is.
- **knot-update: 0.63.0 changelog entry already in place** (written
  during Phases 0–5) and verified complete; all older entries are
  historical and correct as-is. Only the metadata sweep touched this
  skill's neighbours; knot-update itself needed no edits.
- **knot-create second stale section** (beyond the phase's list): the
  "event subscriptions" walkthrough carried its own dispatch-directory
  flow, layout diagram, and a project-layout tree — all reworked; the
  three present-tense `loom-log`/`rig-log` mentions re-pointed to the
  service log (the 0.41.0-retired JSONL files). Historical
  `.loom-log`/`.rig-log` notes elsewhere (knot-update, glossary,
  knot-start, knot-inspect, knot-analyst) are migration records and
  stand.
- **glossary `Knot-state` term**: the old "per-knot file" entry was
  already stale (knot state is derived in-memory into
  `state.json`); reworded to the `loams[].knots[]` entry.
- **knot-start**: the "What the Service Writes" table gained the
  `tie-offs.log` row (missing since the 0.63.0 cutover); the queue
  recovery narrative was accurate as-is (entries persist on disk and
  re-load).
- **AGENTS.md**: two precision fixes — the architecture blurb now
  names `tie-offs.log` explicitly, and the knot-dispatch one-liner
  names the direct queue write.

## Notes
- This phase is the plan's "cross-repo repointing" note, brought inside
  the plan's scope for the project-level skills (which live in this
  repo's `.agents/skills/` and are deployed to `~/.agents/`). The
  affected **rigs'** `rig/knot-template/knot-tie-off-content.md`
  overrides are per-project copies: they are re-pointed in each rig as
  the deploy test exercises them, not in bulk here.
- Documents only — no `cargo` changes, no `cargo test` requirement; the
  audit grep + the deploy diffs are the static panes, the field test is
  the verdict pane.
