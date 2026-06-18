# Knot

A file-first agent loop orchestration framework you use with your favorite agent and IDE.

Knot's a bit different. Set sail.

## Why Knot

Knot runs in the background and works with your existing agents as an orchestration layer over the top.

Out of the box you get:

- **Version-controlled workflows** — everything is plain text, reviewed through normal git diffs. An agents turn is automatically committed.
- **Goal-seeking agents** — knots read state, compare against a goal, and apply only what's needed (idempotent by design)
- **Composable pipelines** — looms group related tasks, knots wire agents to file-based triggers. You can share them with friends and re-use them accross multiple projects.
- **Token efficiency** — Sure...... it might help? We'll
- **Local Development** - Knot enables smaller contexts by facilitating decompostion of workflows and the orchestration means you can let it run unatended for hours grounded by your specifications.
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

Then tell your agent: *"init a knot rig"* (runs the `knot-init` skill), then *"create a loom called `<name>-loom`"* (runs `knot-create`). Create a file in the strand directory and Knot will trigger the agent automatically.

See the [Getting Started guide](https://knot.hdekker.com/getting-started) for a complete walkthrough.

## Documentation

Full documentation is available at **[knot.hdekker.com](https://knot.hdekker.com)**:

- [Getting Started](https://knot.hdekker.com/getting-started) — install, initialise, and run your first knot
- [Concepts](https://knot.hdekker.com/concepts) — looms, knots, strands, profiles, and the processing pipeline
- [Configuration](https://knot.hdekker.com/configuration) — rig structure, knot definitions, and agent profiles
- [Design Guide](https://knot.hdekker.com/design-guide) — idempotency, naming, responsibility, and feedback loops
- [Workflows](https://knot.hdekker.com/workflows) — review and file-generation patterns with examples
- [API Reference](https://knot.hdekker.com/api-reference) — HTTP endpoints and schemas
- [Troubleshooting](https://knot.hdekker.com/troubleshooting) — common issues and fixes
- [Release Notes](https://knot.hdekker.com/release-notes) — feature history and version notes
