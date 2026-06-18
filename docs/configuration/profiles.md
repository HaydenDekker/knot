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

```yaml
---
name: reviewer
provider: openai
model: gpt-4o
tools:
  - fs
system-prompt: |
  You are a thorough reviewer. Analyse documents carefully and
  provide constructive feedback.
---

# Reviewer Profile

Profile for detailed document reviews.
```

## Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Profile identifier. Must match the filename stem (e.g. `reviewer.md` → `name: reviewer`). |
| `provider` | Yes | LLM provider name (e.g. `openai`, `anthropic`, or a pi provider like `llama-workhorse`). |
| `model` | Yes | Model identifier (e.g. `gpt-4o`, `claude-sonnet-4-20250514`, `qwen3-27b`). |
| `system-prompt` | Yes | The agent's system prompt — persona instructions delivered at every session. |
| `tools` | No | List of tool names (e.g. `fs`, `web`). Defaults to empty list. |
| `timeout` | No | Session timeout in seconds. Defaults to 300 (5 minutes). Set higher for long-running tasks. |

### Timeout Example

For long-running tasks like code generation across many files:

```yaml
---
name: coder
provider: openai
model: gpt-4o
tools:
  - fs
timeout: 600
system-prompt: |
  You are a code generation agent. Take your time to be thorough.
---

# Coder Profile

Extended timeout for code generation tasks.
```

When a session exceeds its timeout, a `TimeoutExceeded` event is
recorded in the rig-log (`rig/.rig-log`) and the tie-off file is
preserved unchanged.

## How Profiles Are Used at Processing Time

When a strand event triggers a knot:

1. The knot's `agent-profile-ref` field is used to load the profile from
   `rig/profiles/{name}.md` — **read fresh from disk each time**.
2. The profile provides: `provider`, `model`, and `tools`.
3. The profile's `system-prompt` is merged with the knot's
   `prompt-template.instructions` to form the full system prompt:

   ```
   {profile system-prompt}

   {knot prompt-template.instructions}
   ```

4. This merged prompt is passed to the agent CLI.

Because profiles are read from disk at processing time, edits to a
profile file take effect on the **next strand event** — no restart of
Knot is needed.

## Managing Profiles

### List All Profiles

```bash
curl http://localhost:3000/profiles
```

### Get a Specific Profile

```bash
curl http://localhost:3000/profiles/reviewer
```

### Create a New Profile

Write a `.md` file to `rig/profiles/`:

```bash
cat > rig/profiles/fast.md << 'EOF'
---
name: fast
provider: openai
model: gpt-4o
system-prompt: |
  You are a fast reviewer. Keep responses concise and direct.
---

# Fast Profile

Lightweight profile for quick reviews.
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

## API Note

The `timeout` field is not included in API responses. To check a
profile's timeout, read the file directly from `rig/profiles/{name}.md`
and inspect the YAML frontmatter.
