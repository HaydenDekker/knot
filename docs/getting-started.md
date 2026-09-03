# Getting Started

## Hello Knot

By the end of this guide you will have a working "Hello Knot" — a
minimal rig that watches a directory for files, processes them through
an agent, and writes a result. It's the simplest complete workflow so
you can see Knot's pieces working together before building something
real.

Give the instructions below to your AI agent — it will handle the
setup end-to-end.

## Prerequisites

Ensure the following are available before you begin:

- **A new project with git initialised** — Knot works alongside git.
  Create a fresh project directory and run `git init` in it. This is
  the workspace where your rig will live.
- **An IDE with file watching and git integration** — You'll be
  watching Knot create and update files in real time, and seeing
  commits appear in your git viewer.
  [VS Code](https://code.visualstudio.com/) works well — its built-in
  file watcher and Source Control panel make it easy to follow along.
- **Rust toolchain** — Knot is a Rust binary. Your agent should
  install it via [rustup](https://rustup.rs/) if not already present.
- **An AI agent CLI** — Knot orchestrates external agents. For the
  `pi` CLI, Knot ships with built-in integration and adapts
  out of the box.
- **An LLM provider** — Any provider supported by your agent CLI
  (OpenAI, Anthropic, local models, etc.).

## Step 1: Clone Knot and Import Skills

Have your agent clone the Knot repository, build the binary, and
install the Knot skills so they are available for use:

```bash
git clone <knot-repo-url>
cd knot
cargo install --path .
```

This places the `knot` binary on your `PATH`.

Next, have your agent copy the Knot skills from the repository into
their production locations in `~/.agents/`. For `pi`, sub-skills go
to `~/.agents/skills-library/` (not auto-discovered — they are read
on demand via the master's routing table) and the master router goes
to `~/.agents/skills/` (the only Knot skill auto-discovered):

```bash
for skill in knot-init knot-start knot-create knot-dispatch knot-inspect
              knot-manage knot-design knot-analyst knot-update
              knot-abstractions; do
  mkdir -p ~/.agents/skills-library/$skill
  cp -r knot/.agents/skills/$skill/. ~/.agents/skills-library/$skill/
done
# Master router
mkdir -p ~/.agents/skills/knot
cp -r knot/.agents/skills/knot/. ~/.agents/skills/knot/
```

Knot also ships a glossary of domain terms:

```bash
cp knot/.agents/skills/knot-init/knot-glossary.md \
   ~/.agents/skills-library/knot-init/knot-glossary.md
```

Verify every copy — `cp` can silently fail:

```bash
for skill in knot-init knot-start knot-create knot-dispatch knot-inspect
              knot-manage knot-design knot-analyst knot-update
              knot-abstractions; do
  diff knot/.agents/skills/$skill/SKILL.md \
       ~/.agents/skills-library/$skill/SKILL.md > /dev/null 2>&1 && \
    echo "$skill: OK" || echo "$skill: FAILED"
done
diff knot/.agents/skills/knot/SKILL.md \
     ~/.agents/skills/knot/SKILL.md > /dev/null 2>&1 && \
  echo "knot: OK" || echo "knot: FAILED"
```

For other agent CLIs, your agent will need to place these skills
wherever its framework discovers them.

Once installed, the agent has access to the Knot skills and can
proceed with the remaining steps using them directly.

## Step 2: Initialise the Rig

Ask your agent to run the `knot-init` skill. This will:

1. Create the `rig/` directory and `rig/profiles/` subdirectory
2. If no profiles exist, read available models from
   `~/.pi/agent/models.json` and create a default profile at
   `rig/profiles/default.md`
3. Verify setup by reading `tie-offs/<rig>/state.json`
4. Report the current state back to you

The `knot-init` skill is idempotent — safe to run multiple times.

Watch in your IDE as the agent creates `rig/`, `rig/profiles/`, and
the default profile file.

## Step 3: Start Knot

Run the Knot binary from your project directory:

```bash
knot
```

To keep a record of the run, redirect its output to the service log —
Knot's structured logs (`tie-offs/<rig>/.rig-log` and the loom-logs) are
**cleared at every startup**, so stderr is the only thing that shows what
happened across restarts. Append, never overwrite:

```bash
mkdir -p tie-offs/rig
nohup knot >> tie-offs/rig/knot-service.log 2>&1 &
echo $! > tie-offs/rig/knot-service.pid
```

An agent should use the `knot-start` skill, which does exactly this and
stops the service with `kill -INT $(cat tie-offs/rig/knot-service.pid)`
(Knot handles SIGINT only — a plain `kill` skips the queue drain).

Knot will:

1. Auto-discover the `rig/` directory
2. Scan for looms (any `*-loom/` subdirectory inside `rig/`)
3. Parse knot definition files and agent profiles
4. Start watching strand directories for file changes
5. Begin writing `tie-offs/<rig>/state.json` every 5 seconds

To verify Knot is running, check that `tie-offs/<rig>/state.json` exists and
is being updated:

```bash
watch -n 2 'cat tie-offs/rig/state.json | python3 -m json.tool'
```

You should see the file contain loom and profile information.

## Step 4: Create the "Hello Knot" Loom

A **loom** is a directory ending in `-loom` inside `rig/`. It contains
**knot** definition files (`.md` files with YAML frontmatter).

Ask your agent to create the loom using natural language. For example:

> Create a new loom called `hello-loom`. It must greet the person
> named in the input file — write a short, friendly welcome message.
> Put the strands in the `greeting/` folder.

The agent runs the `knot-create` skill behind the scenes. When it's
done, you should see the new `rig/hello-loom/` directory appear in
your IDE's file tree, containing the knot definition file.

**Can't see the file?** Ask your agent to run `knot-inspect` to
debug the rig state — it will report registered looms, knots, and
any issues by reading `tie-offs/<rig>/state.json`.

## Step 5: Run "Hello Knot"

To trigger the knot, create a file in the strand directory. For
example:

```bash
mkdir -p greeting
echo "Alice" > greeting/alice.md
```

Knot accepts **any text file** as a strand — `.md`, `.rs`, `.json`,
`.py`, `.txt`, etc. Binary files are silently ignored.

Watch for two things:

1. **The tie-off file appears** — Knot's file watcher detects the
   new strand and triggers the knot. The agent runs and writes its
   result to `tie-offs/rig/hello-loom/tie-off-hello.md`. You should see this
   file appear in your IDE, containing the agent's greeting for
   Alice.

2. **A git commit appears** — By default, Knot creates a git commit
   after each successful tie-off write. Check your git viewer
   (Source Control panel in VS Code) — you should see a new commit
   with the result file. To opt out per-knot, set `git-versioned: false`
   in the knot's frontmatter.

That's it. Hello Knot is working.

In your agent of choice, open the session history and explore how your
input was bundled and passed to your agent and how the response was
routed to the knot's tie-off.

## Next Steps

- **[Concepts](concepts.md)** — Understand Knot's architecture
- **[Configuration: Profiles](configuration/profiles.md)** — Configure agents
- **[Configuration: Knots](configuration/knots.md)** — Define processing knots
- **[Configuration: Rig Structure](configuration/rig-structure.md)** — Rig layout and state
- **[Design Guide](design-guide.md)** — Best practices for knot design
- **[Troubleshooting](troubleshooting.md)** — Common issues and fixes

### Working with Skills

Once your rig is running, use the Knot skills to manage it:

| Skill | What it does | When to use |
|-------|-------------|-------------|
| **knot-create** | Create, modify, delete looms and knots | Setting up new workflows |
| **knot-start** | Start, stop, restart the service; capture the service log | Getting the rig running, reading what happened across restarts |
| **knot-dispatch** | Trigger knots into action | Starting processing manually |
| **knot-inspect** | View rig state, looms, knots, profiles | Checking current status |
| **knot-manage** | Review completed work, interaction chains | Quality review of output |
| **knot-analyst** | Analyse rig productivity and blockers | Diagnosing health and progress |
| **knot-design** | Design looms and knots | Planning new workflows |
| **knot-update** | Migrate documents between versions | After Knot binary updates |
