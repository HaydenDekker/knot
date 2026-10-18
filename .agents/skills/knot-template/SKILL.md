---
name: knot-template
description: "Customise the tie-off content section Knot injects into producer agents (the tie-off's header/narrative content and the event-content guide — the description, body, and additional custom properties). The tie-off contract, event format, and rules are hard-coded into the Knot binary and cannot be customised. Explains the mechanism (hard-coded core → one per-rig content-section link, pulled fresh every new session) and provides the content template (knot-tie-off-content.md) plus the shipped contract for reference (knot-tie-off.md). Invoked to add a per-project override: copies the content template into the project's rig and links it in rig/.workspace-agent-config.yaml. USE FOR: customise knot instructions, tie-off content, tie-off header, event content, event description, add knot template, knot template, tie-off template, override tie-off content, override event content, knot config, workspace-agent-config, instruction file. DO NOT USE FOR: creating looms or knots (use knot-create), starting or stopping the service (use knot-start), inspecting rig state (use knot-inspect), changing the hard-coded contract (the tie-off contract, event format, and rules — that is a Knot release, not a project override), or changing the shipped default text (a Knot release — edit the templates in the Knot repository)."
license: MIT
metadata:
  author: Knot Team
  version: "2.1.0"
  compatibility: "Knot 0.60.0+"
---

# Knot template (tie-off content section)

Knot injects a `# Subscriber Events` block into every producer knot's
prompt. It has a **hard-coded core** and **one exposed content
section**:

| Part | Template file (this skill) | Customisable per rig? |
|---|---|---|
| **Hard-coded core** — `## Tie-Off Contract` (the final response *is* the tie-off; never read/write `tie-offs/`), `## Event Format` (the fixed `event:` / `occurred:` / `timestamp:` block), `## Rules` (one structured block per subscriber event — always, even when `occurred: false`) | [`knot-tie-off.md`](knot-tie-off.md) — the shipped contract, **reference only** | **No** — compiled into the Knot binary; a rig can never replace it |
| **Tie-off content section** — `## Tie-Off Content` (the tie-off's header/narrative content) + `## Event Content Guide` (how to fill in `description`, the body, and additional custom properties) | [`knot-tie-off-content.md`](knot-tie-off-content.md) — the default content section, and the starting point for a project override | **Yes** — the only exposed part |

## How the mechanism works

- **Hard-coded core, always present** — the contract, event format, and
  rules are compiled into the binary (from `knot-tie-off.md`, the single
  source of truth). Every rig gets them; no config can replace them.
- **Per-rig content-section override (the config link)** —
  `rig/.workspace-agent-config.yaml` carries one optional key. When
  set, the linked file's **full content** replaces the default content
  section for that rig — the core stays untouched:

  ```yaml
  # Optional full replacement for the default tie-off content section
  # (the tie-off header/narrative content + the event-content guide).
  # File reference: absolute path, relative to the rig directory, ~/...,
  # or a file:// URL. http(s):// is refused (Knot is local-first).
  tie-off-content-instruction-file: knot-template/knot-tie-off-content.md
  ```

- **Pulled fresh per new session** — the config file and the linked
  file are re-read at every new agent session (per strand execution);
  nothing is cached. Editing a linked file takes effect on the next
  strand — no service restart needed.
- **Fallback** — a missing/unreadable linked file never blanks the
  content section: Knot logs a `[KNOT][EVENT] type=CONFIG` service-log
  line and injects the hard-coded default content section.
- **Retired 0.59.0 keys** — `tie-off-instruction-file` and
  `event-content-instruction-file` are no longer honoured (the contract
  can no longer be overridden, and the event-content guide moved into
  the content section). A leftover key produces a service-log warning
  per session. Migration: remove `tie-off-instruction-file`; re-link
  any custom event-content file under
  `tie-off-content-instruction-file`.
- **The service writes nothing into the rig** — it only reads the link.
  The files below are created by *you* (the agent), per the procedure.

## Procedure: add a project override

1. **Locate the rig** — the rig directory is `rig/` at the project root
   (verify `rig/.workspace-agent-config.yaml` exists; if the rig is
   elsewhere, use its actual path).
2. **Copy the content template** — copy the content template from
   **this skill directory** (the directory containing this `SKILL.md`)
   into the project. Do **not** copy `knot-tie-off.md` — it is the
   hard-coded contract, for reference only:

   ```bash
   mkdir -p <rig>/knot-template
   cp <this-skill-dir>/knot-tie-off-content.md <rig>/knot-template/knot-tie-off-content.md
   ```

3. **Link it in the config** — merge the key into
   `rig/.workspace-agent-config.yaml` (read the file first; **never
   clobber** the existing keys such as `agent-adapter`):

   ```yaml
   tie-off-content-instruction-file: knot-template/knot-tie-off-content.md
   ```

   (value relative to the rig directory — the `knot-template/`
   directory created in step 2)
4. **Customise** — edit `<rig>/knot-template/knot-tie-off-content.md`
   as the user wants. It is a full replacement of the **content
   section only**: whatever is in the file is exactly what the agent
   sees in place of the default `## Tie-Off Content` /
   `## Event Content Guide` sections. The hard-coded core (contract,
   event format, rules) is always injected in addition — there is no
   way to override it, and that is deliberate.
5. **Verify** — read the config back. The next new agent session picks
   up the override automatically (check a producer's prompt or the
   service log after the next strand if the user wants confirmation).

## Notes

- **Content only, not mechanics** — the linked file replaces the
  content section; it does not (and cannot) replace the event block
  format or the emit-always rules. Event enforcement parses the
  agent's *response* against the structured block format, and the
  agent always sees the canonical format — the two can no longer
  diverge through a rig override.
- **Local files only** — `http(s)://` links are refused with a
  service-log warning; copy the file locally instead.
- **Per-rig scope** — the config is per-rig; there is no per-knot
  override.
- The shipped default texts live in the Knot repository (this skill's
  directory is their source); to change the default for *all* rigs,
  update the Knot binary — not these project files.
