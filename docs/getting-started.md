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
  out of the box. For other agent CLIs (e.g. `claude`, `cursor`),
  your agent will need to configure its own adapter to communicate
  with Knot's HTTP interface.
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
its skill discovery path. For `pi`, copy them to `~/.agents/skills/`:

```bash
cp -r knot/.agents/skills/knot-init     ~/.agents/skills/
cp -r knot/.agents/skills/knot-create   ~/.agents/skills/
cp -r knot/.agents/skills/knot-inspect  ~/.agents/skills/
```

For other agent CLIs, your agent will need to place these skills
wherever its framework discovers them.

Once installed, the agent has access to the Knot skills and can
proceed with the remaining steps using them directly.

## Step 2: Initialise the Rig

Ask your agent to run the `knot-init` skill. This will:

1. Start the Knot server (if not already running) and confirm it
   is reachable via `GET /health`
2. Read the rig configuration from `GET /config/rig`
3. Create the `rig/profiles/` directory structure
4. If no profiles exist, read available models from
   `~/.pi/agent/models.json` and create a default profile at
   `rig/profiles/default.md`
5. Verify everything via `GET /profiles` and `GET /looms`
6. Report the current state back to you

The `knot-init` skill is idempotent — safe to run multiple times.

Watch in your IDE as the agent creates `rig/`, `rig/profiles/`, and
the default profile file.

## Step 3: Create the "Hello Knot" Loom

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
any issues.

## Step 4: Run "Hello Knot"

To trigger the knot, create a file in the strand directory. For
example:

```bash
mkdir -p greeting
echo "Alice" > greeting/alice.md
```

Watch for two things:

1. **The tie-off file appears** — Knot's file watcher detects the
   new strand and triggers the knot. The agent runs and writes its
   result to `rig/tie-offs/hello-loom/hello/`. You should see this
   file appear in your IDE, containing the agent's greeting for
   Alice.

2. **A git commit appears** — The agent commits the tie-off to git.
   Check your git viewer (Source Control panel in VS Code) — you
   should see a new commit with the result file.

That's it. Hello Knot is working.

In your agent of choice, open the session history and explore how your
input was bundled and passed to your agent and how the response was
routed to the knots tie-off.

## Next Steps

- **[Concepts](concepts.md)** — Understand Knot's architecture
- **[Configuration: Profiles](configuration/profiles.md)** — Configure agents
- **[Configuration: Knots](configuration/knots.md)** — Define processing knots
- **[Design Guide](design-guide.md)** — Best practices for knot design
- **[API Reference](api-reference.md)** — Complete endpoint documentation
- **[Troubleshooting](troubleshooting.md)** — Common issues and fixes
