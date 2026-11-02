---
name: knot-template
description: "Customise the two instruction documents Knot injects into agent prompts: the tie-off instruction (the tie-off's header/narrative content — injected into EVERY knot, every new session, unconditional) and the event-content guide (how to fill in event blocks — injected only when the knot has subscriber events). The hard-coded cores (the universal tie-off contract sentence; the event acknowledgement, format, and rules) are compiled into the Knot binary and cannot be customised. Explains the mechanism (hard-coded core split + two per-rig instruction links, pulled fresh every new session, per-section fallback) and provides the two default documents (knot-tie-off-content.md, knot-event-content.md) plus the two shipped core references (knot-tie-off-contract.md, knot-event-contract.md). Invoked to add a per-project override: copies both templates into the project's rig and links the two keys in rig/.workspace-agent-config.yaml. Covers the migration from the 0.60.0–0.64.x combined key (tie-off-content-instruction-file — retired in Knot 0.65.0)."
license: MIT
metadata:
  author: Knot Team
  version: "3.0.0"
  compatibility: "Knot 0.65.0+"
---

# Knot template (the two instruction documents)

Knot injects up to **two** blocks at the start of every knot's prompt.
Each block has a **hard-coded core** (compiled into the binary, never
overridable per rig) and **one overridable document** with a **fixed
injection scope**:

| Block | Hard-coded core (reference file) | Overridable document (template file) | Injection scope | Config key |
|---|---|---|---|---|
| `# Tie-Off` — the agent's final response *is* the tie-off | [`knot-tie-off-contract.md`](knot-tie-off-contract.md) — `## Tie-Off Contract`: the universal sentence ("never write or read directly from `tie-offs/` files"), **reference only** | [`knot-tie-off-content.md`](knot-tie-off-content.md) — `## Tie-Off Content`: the tie-off's header/narrative content | **Every knot, every new session — unconditional** (leaf knots included) | `tie-off-instruction-file` |
| `# Subscriber Events` — the events a knot must acknowledge in its tie-off | [`knot-event-contract.md`](knot-event-contract.md) — `## Event Acknowledgement` (acknowledge every subscriber event), `## Event Format` (the fixed `event:` / `occurred:` / `timestamp:` block), `## Rules` (one structured block per subscriber event — always, even when `occurred: false`), **reference only** | [`knot-event-content.md`](knot-event-content.md) — `## Event Content Guide`: how to fill in `description`, the body, and additional custom properties | **Only when the knot has subscriber events** (or a self-continuation entry) — i.e. only when the block is built | `event-content-instruction-file` |

## How the mechanism works

- **Hard-coded cores, always present (per scope)** — the universal
  tie-off contract sentence is compiled in from `knot-tie-off-contract.md`
  and reaches **every** knot (the tie-off concern is not scoped to
  producer status). The event acknowledgement / format / rules are
  compiled in from `knot-event-contract.md` and reach **only** knots
  that emit to subscribers. No config can replace either core.
- **Per-rig instruction overrides (two config links)** —
  `rig/.workspace-agent-config.yaml` carries two optional keys. When
  set, each linked file's **full content** replaces the default
  document for **that section only** — the other section and both
  cores stay untouched:

  ```yaml
  # Optional full replacement for the default tie-off instruction
  # (the tie-off header/narrative content). Injected into EVERY knot,
  # every new session.
  tie-off-instruction-file: knot-template/knot-tie-off-content.md

  # Optional full replacement for the default event-content guide
  # (how to fill in event blocks). Injected only when the knot has
  # subscriber events.
  event-content-instruction-file: knot-template/knot-event-content.md
  ```

- **Pulled fresh per new session** — the config file and both linked
  files are re-read (two pulls) at every new agent session (per strand
  execution); nothing is cached. Editing a linked file takes effect on
  the next strand — no service restart needed.
- **Per-section fallback** — a missing/unreadable link never blanks or
  alters its section: Knot logs a `[KNOT][EVENT] type=CONFIG`
  service-log line and injects the embedded default for **that
  section only**. The other section is unaffected.
- **Retired 0.60.0–0.64.x key** — the combined
  `tie-off-content-instruction-file` key is no longer honoured (the
  combined section is split into the two documents above). A leftover
  key still parses and produces a service-log warning per session.
  Migration: see below.
- **The service writes nothing into the rig** — it only reads the
  links. The files below are created by *you* (the agent), per the
  procedure.

## Procedure: add a project override

1. **Locate the rig** — the rig directory is `rig/` at the project root
   (verify `rig/.workspace-agent-config.yaml` exists; if the rig is
   elsewhere, use its actual path).
2. **Copy both templates** — copy the two document templates from
   **this skill directory** (the directory containing this `SKILL.md`)
   into the project. Do **not** copy `knot-tie-off-contract.md` or
   `knot-event-contract.md` — they are the hard-coded cores, for
   reference only:

   ```bash
   mkdir -p <rig>/knot-template
   cp <this-skill-dir>/knot-tie-off-content.md <rig>/knot-template/knot-tie-off-content.md
   cp <this-skill-dir>/knot-event-content.md <rig>/knot-template/knot-event-content.md
   ```

3. **Link them in the config** — merge the two keys into
   `rig/.workspace-agent-config.yaml` (read the file first; **never
   clobber** the existing keys such as `agent-adapter`):

   ```yaml
   tie-off-instruction-file: knot-template/knot-tie-off-content.md
   event-content-instruction-file: knot-template/knot-event-content.md
   ```

   (values relative to the rig directory — the `knot-template/`
   directory created in step 2)
4. **Customise** — edit the linked files as the user wants. Each is a
   full replacement of **its section only**: whatever is in the file is
   exactly what the agent sees in place of that section's default. The
   hard-coded cores are always injected in addition — there is no way
   to override them, and that is deliberate.
5. **Verify** — read the config back. The next new agent session picks
   up the overrides automatically (check a knot's prompt or the service
   log after the next strand if the user wants confirmation).

## Migration (0.60.0–0.64.x rigs)

Knot 0.60.0–0.64.x carried **one** overridable content section under
`tie-off-content-instruction-file`. Knot 0.65.0 splits it into the two
documents above. If the rig's config still carries the old key:

1. **Split the linked file at the `## Event Content Guide` heading** —
   the top half (the `## Tie-Off Content` section) stays
   `knot-template/knot-tie-off-content.md`; the bottom half becomes
   `knot-template/knot-event-content.md`.
2. **Replace the key** — in the rig's
   `.workspace-agent-config.yaml` (for a shim rig: the shim's local
   config *and* the external rig's config, both of which may carry it):

   ```yaml
   tie-off-instruction-file: knot-template/knot-tie-off-content.md
   event-content-instruction-file: knot-template/knot-event-content.md
   ```

   and remove `tie-off-content-instruction-file`.
3. **Legacy 0.59.0 links** — a `tie-off-instruction-file` linked to a
   0.59.0-era **full-contract** file is honoured again (the 0.60.0
   deprecation warning is removed), but its content now replaces only
   the tie-off **content** section. Point it at a content-only file (or
   the default template) so the agent is not shown the old full
   contract as content.

The binary changes nothing in the rig — the migration is
agent/user-driven.

## Notes

- **Content only, not mechanics** — the linked files replace the
  documents; they do not (and cannot) replace the event block format or
  the emit-always rules. Event enforcement parses the agent's *response*
  against the structured block format, and the agent always sees the
  canonical format — the two can no longer diverge through a rig
  override.
- **Local files only** — `http(s)://` links are refused with a
  service-log warning; copy the file locally instead.
- **Per-rig scope** — the config is per-rig; there is no per-knot
  override.
- **Fixed injection scopes** — the tie-off instruction reaches every
  knot unconditionally; the event-content guide reaches only
  subscriber-bearing knots. Do not expect the event-content guide in a
  leaf knot's prompt (and do not put tie-off-only content in
  `knot-event-content.md` — it is invisible to leaf knots).
- The shipped default texts live in the Knot repository (this skill's
  directory is their source); to change the default for *all* rigs,
  update the Knot binary — not these project files.
