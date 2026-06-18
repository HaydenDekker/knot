# Getting Started

This guide walks you through installing Knot, initialising your first rig,
and verifying everything works.

## Prerequisites

- **Rust toolchain** — Knot is a Rust binary. Install via
  [rustup](https://rustup.rs/).
- **An AI agent CLI** — Knot orchestrates external agents. It does not
  include one. You bring your own (e.g. `pi`, `claude`, `cursor`).
- **An LLM provider** — Any provider supported by your agent CLI
  (OpenAI, Anthropic, local models, etc.).

## Install Knot

### From Source

```bash
git clone <knot-repo-url>
cd knot
cargo build --release
```

The binary is at `target/release/knot`.

### Install Globally

```bash
cargo install --path .
```

This places `knot` on your `PATH`.

## Start the Server

Run Knot from your project directory (the directory that will contain
your `rig/` folder):

```bash
knot
```

By default, Knot binds to `127.0.0.1:3000` and looks for a `rig/`
directory in the current working directory.

Verify the server is running:

```bash
curl http://localhost:3000/health
# Expected: ok
```

## Initialise Your First Rig

A **rig** is Knot's top-level configuration container. Initialise it by
creating the required directory structure:

```bash
mkdir -p rig/profiles
```

Knot auto-discovers this directory on startup — no HTTP registration is
needed.

### Create a Default Profile

Profiles define which agent runs and how. Create your first profile at
`rig/profiles/default.md`:

```yaml
---
name: default
provider: openai
model: gpt-4o
system-prompt: |
  You are a helpful AI assistant. Follow the instructions
  provided in each task.
---

# Default Profile

My default agent profile.
```

Replace `provider` and `model` with values matching your agent CLI
configuration. See [Configuration: Profiles](configuration/profiles.md)
for all available fields.

Verify the profile is discovered:

```bash
curl http://localhost:3000/profiles
```

Expected response (JSON array):

```json
[
  {
    "name": "default",
    "provider": "openai",
    "model": "gpt-4o",
    "tools": [],
    "system_prompt": "You are a helpful AI assistant..."
  }
]
```

## Create Your First Loom and Knot

A **loom** is a directory ending in `-loom` inside `rig/`. It contains
**knot** definition files (`.md` files with YAML frontmatter).

Create a loom:

```bash
mkdir -p rig/prd-review-loom
```

Create a knot definition at `rig/prd-review-loom/goals-review.md`:

```yaml
---
name: goals-review
agent-profile-ref: default
strand-dir: "project/prds"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the goals section for clarity and measurability.
---

# Goals Review Knot

Reviews PRD goals sections.
```

This knot will watch the `project/prds/` directory for file changes and
process them using the `default` profile.

Verify Knot has discovered the loom:

```bash
curl http://localhost:3000/looms
```

Expected response:

```json
[
  {
    "id": {"0": "prd-review-loom"},
    "knot_count": 1
  }
]
```

## Trigger Processing

Place a file (a **strand**) in the knot's `strand-dir`. For example:

```bash
mkdir -p project/prds
cat > project/prds/goals.md << 'EOF'
# Goals

- Improve code review turnaround time
- Reduce production incidents
EOF
```

Knot's file watcher detects the new file and triggers the knot. The
agent runs, and the result (a **tie-off**) is written to
`rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md`.

Check the knot's processing status:

```bash
curl http://localhost:3000/looms/prd-review-loom/knots/goals-review
```

## View Activity

See what happened during processing:

```bash
curl http://localhost:3000/looms/prd-review-loom/activity
```

This returns the loom's activity log — a chronological list of events
including knot registration, processing start/completion, and any
errors.

## Explore the API

Knot serves a Swagger UI with full API documentation at:

```
http://localhost:3000/swagger-ui
```

This interactive interface lets you browse all endpoints, inspect
response schemas, and try requests directly in your browser.

## Next Steps

- **[Concepts](concepts.md)** — Understand Knot's architecture
- **[Configuration: Profiles](configuration/profiles.md)** — Configure agents
- **[Configuration: Knots](configuration/knots.md)** — Define processing knots
- **[Design Guide](design-guide.md)** — Best practices for knot design
- **[API Reference](api-reference.md)** — Complete endpoint documentation
- **[Troubleshooting](troubleshooting.md)** — Common issues and fixes
