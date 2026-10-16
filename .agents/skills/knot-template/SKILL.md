---
name: knot-template
description: "Customise the prompt instructions Knot injects into producer agents (the tie-off contract and the event-content guide). Explains the mechanism (hard-coded default → per-rig config link, pulled fresh every new session) and provides the two template files (knot-tie-off.md, knot-event-content.md). Invoked to add a per-project override: copies the templates into the project's rig and links them in rig/.workspace-agent-config.yaml. USE FOR: customise knot instructions, tie-off instruction, event content instruction, add knot template, knot template, tie-off template, override tie-off, override event content, knot config, workspace-agent-config, instruction file. DO NOT USE FOR: creating looms or knots (use knot-create), starting or stopping the service (use knot-start), inspecting rig state (use knot-inspect), changing the shipped default text (that is a Knot release — edit the templates in the Knot repository)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.59.0+"
---

# Knot template (tie-off / event-content instructions)

The two instruction texts Knot injects into every producer knot's prompt
(the `# Subscriber Events` block) are:

| Instruction | Template file (this skill) | Owns |
|---|---|---|
| **Tie-off contract** | [`knot-tie-off.md`](knot-tie-off.md) | The final response *is* the tie-off; one structured event block per subscriber event; frontmatter format; `occurred` semantics; the "never read/write `tie-offs/` directly" rule |
| **Event content guide** | [`knot-event-content.md`](knot-event-content.md) | How to fill in each block: the `description` field, the narrative body, and the extra fields a subscriber declares in its `event_description` |

## How the mechanism works

- **Hard-coded default** — the Knot binary ships these two texts compiled
  in (they are the single source of truth for the default). A rig with no
  override always gets the default.
- **Per-rig override (the config link)** — `rig/.workspace-agent-config.yaml`
  carries two optional keys. When set, the linked file's **full content**
  replaces the corresponding default for that rig:

  ```yaml
  # Optional full replacement for the hard-coded instruction defaults.
  # File reference: absolute path, relative to the rig directory, ~/...,
  # or a file:// URL. http(s):// is refused (Knot is local-first).
  tie-off-instruction-file: knot-template/knot-tie-off.md
  event-content-instruction-file: knot-template/knot-event-content.md
  ```

- **Pulled fresh per new session** — the config file and the linked file
  are re-read at every new agent session (per strand execution); nothing
  is cached. Editing a linked file takes effect on the next strand — no
  service restart needed.
- **Fallback** — a missing/unreadable linked file never blanks the
  contract: Knot logs a `[KNOT][EVENT] type=CONFIG` service-log line and
  injects the hard-coded default.
- **The service writes nothing into the rig** — it only reads the link.
  The files below are created by *you* (the agent), per the procedure.

## Procedure: add a project override

1. **Locate the rig** — the rig directory is `rig/` at the project root
   (verify `rig/.workspace-agent-config.yaml` exists; if the rig is
   elsewhere, use its actual path).
2. **Copy the templates** — copy the two template files from **this skill
   directory** (the directory containing this `SKILL.md`) into the
   project:

   ```bash
   mkdir -p <rig>/knot-template
   cp <this-skill-dir>/knot-tie-off.md <rig>/knot-template/knot-tie-off.md
   cp <this-skill-dir>/knot-event-content.md <rig>/knot-template/knot-event-content.md
   ```

3. **Link them in the config** — merge the two keys into
   `rig/.workspace-agent-config.yaml` (read the file first; **never
   clobber** the existing keys such as `agent-adapter`):

   ```yaml
   tie-off-instruction-file: knot-template/knot-tie-off.md
   event-content-instruction-file: knot-template/knot-event-content.md
   ```

   (values relative to the rig directory — the `knot-template/` directory
   created in step 2)
4. **Customise** — edit `<rig>/knot-template/*.md` as the user wants.
   These are full replacements: whatever is in the file is exactly what
   the agent sees in place of the default.
5. **Verify** — read the config back. The next new agent session picks up
   the override automatically (check a producer's prompt or the service
   log after the next strand if the user wants confirmation).

## Notes

- **Full replacement, not a merge** — a linked file that omits part of the
  default (e.g. the event block format) means the agent no longer sees
  that part. Event enforcement parses the agent's *response* against the
  structured block format, so keep the `## Event Format` mechanics intact
  unless the user explicitly accepts that trade-off.
- **Local files only** — `http(s)://` links are refused with a service-log
  warning; copy the file locally instead.
- **Per-rig scope** — the config is per-rig; there is no per-knot
  override.
- The shipped default text itself lives in the Knot repository (the
  `knot-template` skill is its source); to change the default for *all*
  rigs, update the Knot binary — not these project files.
