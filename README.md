# Knot

A file-first agent loop orchestration harness you use with your favorite agent and IDE.

Knot's a bit different. Set sail.

## Why Knot

Knot runs in the background and works with your existing agents as an orchestration layer over the top.

Out of the box you get:

- **Version-controlled workflows** — everything is plain text, reviewed through normal git diffs. An agents turn is automatically committed.
- **Goal-seeking agents** — knots read state, compare against a goal, and apply only what's needed (idempotent by design)
- **Composable pipelines** — looms group related tasks, knots wire agents to file-based triggers. You can share them with friends and re-use them accross multiple projects.
- **Token efficiency** — Sure...... it might help? We'll
- **Local Development King** - Knot enables smaller contexts by facilitating decompostion of workflows and the orchestration means you can let it run unatended for hours grounded by your specifications.
- **Long Horizon** - You decompose and tune your workflow iteratively in knot. As it comes together it takes off.
- **Natural Evals** - You rig is standalone, copy it, modify the profiles, rerun, assess and compare.
- **HITL Native** - Human In The Loop grounds you agents, they can't read your mind. Create your knots with HITL strands and they'll keep coming back to your truth as and when needed.

## Concepts

| Term | Description |
|------|-------------|
| **Rig** | Your project's Knot configuration — lives at `./rig/` |
| **Loom** | A namespace for a domain of responsibility (e.g. `planning-loom`) |
| **Knot** | A single processing task: agent + prompt + input directory |
| **Strand** | An input file that triggers a knot when changed |
| **Tie-off** | The append-only output log of a knot's work |
| **Profile** | Agent configuration (model, tools, system prompt) |

Read the full [Concepts guide](https://knot.hdekker.com/concepts) for the complete mental model and processing flow.

## Quick Start

```bash
git clone <repo> && cd knot
cargo install --path .
```

Then tell your agent:

1. *"init a knot rig"* — runs `knot-init` (sets up `rig/`, profiles, and installs skills globally)
2. *"create a loom called `<name>-loom`"* — runs `knot-create`
3. *"trigger the knot"* — runs `knot-dispatch` (creates strand files to start processing)
4. *"review the rig's work"* — runs `knot-manage` (examines tie-offs and interaction quality)
5. *"analyse rig health"* — runs `knot-analyst` (assesses productivity, blockers, and progress)

See the [Getting Started guide](https://knot.hdekker.com/getting-started) for a complete walkthrough.

## Documentation

Full documentation is available at **[knot.hdekker.com](https://knot.hdekker.com)**:

- [Getting Started](https://knot.hdekker.com/getting-started) — install, initialise, and run your first knot
- [Concepts](https://knot.hdekker.com/concepts) — looms, knots, strands, profiles, and the processing pipeline
- [Configuration](https://knot.hdekker.com/configuration) — rig structure, knot definitions, and agent profiles
- [Design Guide](https://knot.hdekker.com/design-guide) — idempotency, naming, responsibility, and feedback loops
- [Workflows](https://knot.hdekker.com/workflows) — review and file-generation patterns with examples
- [Troubleshooting](https://knot.hdekker.com/troubleshooting) — common issues and fixes
- [Release Notes](https://knot.hdekker.com/release-notes) — feature history and version notes
