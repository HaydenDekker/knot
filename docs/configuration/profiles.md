# Configuration: Profiles

Agent profiles define which AI agent runs and how. They specify the LLM
provider, model, available tools, and the system prompt (persona
instructions). Profiles are shared — multiple knots can reference the
same profile.

## File Format

Profiles are `.md` files with YAML frontmatter stored in
`rig/profiles/{name}.md`. The file stem (without `.md`) is the profile's
identifier.

### Example

`rig/profiles/reviewer.md`:

```markdown
---
name: reviewer
provider: openai
model: gpt-4o
tools:
  - fs
---

You are a thorough reviewer. Analyse documents carefully and
provide constructive feedback.
```

## Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Profile identifier. Must match the filename stem (e.g. `reviewer.md` → `name: reviewer`). |
| `provider` | Yes | LLM provider name (e.g. `openai`, `anthropic`, or a pi provider like `llama-workhorse`). |
| `model` | Yes | Model identifier (e.g. `gpt-4o`, `claude-sonnet-4-20250514`, `qwen3-27b`). |
| `tools` | No | List of tool names (e.g. `fs`, `web`). Defaults to empty list. |
| `timeout` | No | Session timeout in seconds. Defaults to 300 (5 minutes). Set higher for long-running tasks. |

### Markdown Body

The text after the closing `---` is the agent's system prompt
(persona instructions). This is the primary content of the profile.

The body must not be empty or contain only whitespace.

### Timeout Example

For long-running tasks like code generation across many files:

```markdown
---
name: coder
provider: openai
model: gpt-4o
tools:
  - fs
timeout: 600
---

You are a code generation agent. Take your time to be thorough.
```

When a session exceeds its timeout, a `TimeoutExceeded` event is
recorded in the rig-log (`rig/.rig-log`) and the tie-off file is
preserved unchanged.

## How Profiles Are Used at Processing Time

When a strand event triggers a knot:

1. The knot's `agent-profile-ref` field is used to load the profile from
   `rig/profiles/{name}.md` — **read fresh from disk each time**.
2. The profile provides: `provider`, `model`, and `tools`.
3. The profile's markdown body is merged with the knot's markdown
   body to form the full prompt:

   ```
   {profile body}

   {knot body}
   ```

4. This merged prompt is passed to the agent CLI.

Because profiles are read from disk at processing time, edits to a
profile file take effect on the **next strand event** — no restart of
Knot is needed.

## Managing Profiles

### List All Profiles

Read `rig/state.json` to see all registered profiles:

```bash
cat rig/state.json | python3 -m json.tool
```

### Create a New Profile

Write a `.md` file to `rig/profiles/`:

```bash
cat > rig/profiles/fast.md << 'EOF'
---
name: fast
provider: openai
model: gpt-4o
---

You are a fast reviewer. Keep responses concise and direct.
EOF
```

Knot discovers it automatically via its file watcher.

### Modify a Profile

Edit the `.md` file directly. Changes are picked up on the next strand
event.

### Delete a Profile

Remove the file:

```bash
rm rig/profiles/fast.md
```

Knot discovers the removal automatically. Note: any knots referencing
the deleted profile will fail on their next processing run with a
`ProfileNotFound` error.

### Using Skills

Ask your agent to manage profiles using `knot-create`:

- *"create a profile called `fast` with openai/gpt-4o"*
- *"list all profiles"* — runs `knot-inspect` to read `rig/state.json`
- *"update the default profile timeout to 600s"*

## Session Resume

If an agent invocation fails (timeout, network error, process crash),
Knot automatically attempts to resume the session:

- Up to **10 retries** per strand event
- **10-second delay** between retries (for network recovery)
- Retries stop when the profile's **timeout budget** is nearly
  exhausted (minimum 5 seconds remaining)
- Each retry appends "please continue" to the session
- Session resume events are logged as `SessionResumed` in the loom-log

This makes Knot resilient to transient failures without losing agent
context.
