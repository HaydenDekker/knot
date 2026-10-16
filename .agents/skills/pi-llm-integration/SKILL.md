---
name: pi-llm-integration
description: "Use the local pi CLI as an LLM agent from your own code. A quick, high-level reference on the three communication modes pi exposes (plain CLI / text, JSON event stream, and JSON-RPC over stdin/stdout), how to confirm pi is installed locally, and where to find pi's documentation. USE FOR: pi as an agent, invoke llm from code, spawn pi subprocess, pi cli integration, pi json mode, pi rpc mode, pi streaming events, steer a pi session, pi token usage, pi session id, pi documentation, is pi installed, pi version, pick a pi comms mode. DO NOT USE FOR: Knot rig/loom/knot work (use the knot master) or building/extending pi itself."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "pi >= 0.78 (0.85.1 verified)"
---

# pi LLM Integration

`pi` is a local AI-coding agent. To use it as an LLM from your own
process, you spawn the `pi` binary as a subprocess and talk to it over
its CLI. There are **three communication modes**, selected by the
`--mode` flag; they form a ladder of increasing integration (and
increasing control):

```
text (default)  →  json  →  rpc
  fire & read     observability   two-way control
```

This skill is a quick reference. For full protocol detail, read pi's
own docs (paths below) — do not guess at event/command shapes.

---

## Confirm pi is installed

```bash
which pi          # → e.g. /home/<user>/.nvm/versions/node/v26.2.0/bin/pi
pi --version      # → 0.85.1
pi --help         # full CLI flag reference
```

- **Present & runnable** = `which pi` resolves and `pi --version`
  prints a semver. `pi --help` lists every flag (`--mode`, `--print`,
  `--model`, `--session`, …).
- pi is an npm package (`@earendil-works/pi-coding-agent`) installed
  globally. If `which pi` is empty, it is not on `PATH`.
- RPC needs a modern pi: `agent_settled` and the full command set
  assume pi ≥ 0.78.

---

## The three communication modes

All three are the same binary; only `--mode` changes the wire format.
Model/thinking/tools are chosen the same way in every mode via CLI
flags (`--model <pattern>`, `--thinking <level>`, `--tools <list>`).
The prompt is delivered either as a CLI arg (`-p "..."`) or on **stdin**
(for longer content, and always for `rpc`).

| Mode | Flag | You send | You get back | Can you steer a running session? |
|------|------|----------|--------------|----------------------------------|
| **CLI / text** | *(default)* | prompt (arg or stdin) | raw text on stdout | No — one shot, read text, exit |
| **JSON** | `--mode json` | prompt (arg or stdin) | newline-delimited JSON **events** on stdout | No — observe only |
| **RPC** | `--mode rpc` | JSON **commands** on stdin | JSON **responses** + JSON **events** on stdout | **Yes** — full two-way control |

Knot uses all three in separate adapters — read them as working
reference implementations:

| Mode | Knot adapter |
|------|--------------|
| CLI / text | `src/adapters/pi_stdio.rs` |
| JSON | `src/adapters/pi_json.rs` |
| RPC | `src/adapters/pi_rpc.rs` |

### 1. CLI / text — fire and read

The simplest path. Non-interactive (`--print` / `-p`) runs the prompt
and exits; stdout is the answer.

```bash
pi -p "Summarise the risks in this plan" my-plan.md
# or feed the prompt on stdin:
echo "Explain this API" | pi -p
```

- Read stdout as the response; check the exit code for errors.
- `@file` args inline a file's contents into the prompt: `pi -p "…" @notes.md`.
- No session id or token usage is surfaced.
- Use it when you just need a one-shot answer and want zero parsing.

### 2. JSON — structured observability

`--mode json` streams every session event as one JSON object per line
(JSON-L) on stdout. The **first line** is a session header; the rest are
lifecycle events.

```bash
pi --mode json -p "List files" 2>/dev/null | jq -c 'select(.type=="agent_end")'
```

What you can extract from the stream:

- **Session id** — from the first `{"type":"session","id":"…"}` header.
  Lets you later inspect/resume the exact run (`pi --session-id <id>`).
- **Final answer** — the `agent_end` event carries the full `messages`
  array; take the assistant text whose `stopReason` is `stop`/`length`
  (skip `toolUse` turns).
- **Token usage** — `agent_end` / `message_update` carry a `usage` object
  (`input`, `output`, `cache_read`, `cache_write`, `total`).
- **Compaction** — `compaction_start` / `compaction_end` events.
- **Tool activity** — `tool_execution_start` / `tool_execution_end`.

It is **read-only**: you observe the run, you cannot redirect it mid-way.
Parse stdout line-by-line; if a line is not valid JSON, fall back to
treating stdout as raw text (Knot's JSON adapter degrades this way).

### 3. RPC — two-way control

`--mode rpc` opens a JSON command/response protocol on stdin + a JSON
event stream on stdout. This is the only mode that lets you **steer a
running session**: send a `prompt`/`steer`, sample live context usage,
or manually compact — all while the session is alive.

Framing is strict JSONL: one JSON object per line, `\n`-delimited.
Commands go to **stdin**; `{"type":"response",…}` and events come back on
**stdout**. An optional `id` on a command is echoed on its response for
correlation.

A minimal session looks like:

```
stdin →  {"type":"get_state"}                 # capture sessionId, model, …
stdin →  {"type":"prompt","message":"Do X"}   # start the work
stdout ← events… then {"type":"agent_settled"}  # truly done
stdin →  (close stdin)                        # pi exits on stdin EOF
```

Useful commands (see `docs/rpc.md` for the full list and payloads):

- `prompt`, `steer`, `follow_up` — send / redirect messages. `steer` is
  delivered between the current turn's tool calls and the next LLM call
  (this is how you ask the agent to "wrap up" near a context limit).
- `get_state` — session id, model, streaming/compaction flags.
- `get_session_stats` — token usage + live `contextUsage` (the same value
  pi uses for its footer and compaction decision).
- `compact` — manual compaction (with optional `customInstructions`).
- `abort`, `set_model`, `set_thinking_level`, `bash`, …

Two RPC-specific gotchas worth knowing up front:

- **Close stdin only when the turn is genuinely done** — on
  `agent_settled` (pi ≥ 0.78), not on `agent_end`. pi exits on stdin EOF,
  so closing early cuts off post-`agent_end` work (that is where
  auto-compaction runs).
- **`agent_end` is not "finished"** — it marks one low-level run; retry,
  compaction, and queued continuations can follow. `agent_settled` is the
  real end-of-run signal.

---

## Where to find pi's documentation

pi ships its docs **inside the npm package**
(`@earendil-works/pi-coding-agent`). Two ways to find them:

```bash
# The main README (package root)
npm root -g   # → e.g. /home/<user>/.nvm/versions/node/v26.2.0/lib/node_modules
# docs live in: <npm root>/@earendil-works/pi-coding-agent/docs/

# Or trace the binary to its package
readlink -f "$(which pi)"
```

On this machine that resolves to:
`/home/hayden/.nvm/versions/node/v26.2.0/lib/node_modules/@earendil-works/pi-coding-agent/docs/`

Start with these (relative to the `docs/` dir):

| File | Covers |
|------|--------|
| `usage.md` | CLI, flags, non-interactive `-p`, `@file` |
| `json.md` | JSON event stream: event types, output format, examples |
| `rpc.md` | RPC protocol: every command, responses, events, examples |
| `sessions.md` | Session persistence, resume/fork, `--session-id` |
| `compaction.md` | Context compaction & overflow behaviour |
| `settings.md` / `models.md` / `providers.md` | Config, model registry, provider setup |
| `sdk.md` | Embedding pi from Node/TypeScript (no subprocess) |
| `extensions.md` / `skills.md` | Extending pi |

> **Rule of thumb:** this skill tells you *which* mode to use and *where*
> to look. The authoritative shapes of events and commands live in
> `json.md` and `rpc.md` — read those before writing a parser or driver,
> and re-check after any pi upgrade (the wire format has evolved across
> versions).

---

## Which mode should I pick?

- **Just need an answer, minimal code** → **CLI / text** (`pi -p "…"`).
- **Need the session id, token usage, or the clean final answer** (e.g.
  to log cost or resume later) → **JSON**.
- **Need to steer a long run, sample context, or compact on demand**
  (e.g. a wrap-up trigger near a token limit) → **RPC**.

Default to the *simplest* mode that satisfies the requirement; escalate
only when you need the extra control. In Knot, `pi-json` is the default
adapter and `pi-rpc` is opt-in (required for the per-model
`ctx-wrap-up-limit` wrap-up feature).

---

## Cross-reference

- **Knot adapters** — `src/adapters/pi_stdio.rs`, `pi_json.rs`,
  `pi_rpc.rs` (production reference implementations of the three modes).
- **Knot config** — the adapter is selected per-rig via
  `agent-adapter: pi-stdio | pi-json | pi-rpc` in
  `.workspace-agent-config.yaml` (see `knot` / `knot-create` skills).
- **pi docs** — `docs/usage.md`, `docs/json.md`, `docs/rpc.md` (authoritative).
