# Plan: pi Agent Integration

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan implements **Story 4: Configure Agent Runtime** — Knot constructs and calls the `pi` CLI with the agent profile and prompt template from knot config. The PRD specifies: *"The user configures `agent-config` in the knot with CLI arguments, and Knot parses the knot config to construct the full CLI invocation (provider, model, skills, tools, system prompt)."*

## Problem

The system-integration plan wired all layers together, but the agent runner invokes `pi` with **zero arguments**. The knot's `agent-config` contains only a free-text `goal` string — Knot never constructs a real `pi` CLI invocation from the knot's configuration. This means:

- No LLM provider or model is specified
- The system prompt from the knot's `prompt-template.instructions` is never passed to `pi`
- The strand content is never fed to the agent
- Tie-off files are created but contain empty stdout

The gap is between "we know `pi` is the agent" (hardcoded default) and "we construct the correct `pi` invocation per knot" (what the PRD requires).

## Target

Knot constructs a `pi` CLI invocation from each knot's configuration:

```
pi -p --model <model> --system-prompt "<instructions>" --no-session --no-tools "@<strand_path>"
```

Where `<model>`, `<instructions>`, and strand path come from the knot file. The agent runner passes strand content via `@<path>` (pi's file-include syntax) and/or stdin.

Concretely:

- **AgentConfig** domain value object gains `provider`, `model`, and `tools` fields
- **Knot file parser** reads these fields from frontmatter (`agent-config.provider`, `agent-config.model`, `agent-config.tools`)
- **`ProcessStrand`** builds the `pi` CLI args from the knot's agent config + prompt template
- **Subprocess agent runner** passes the prompt via `@<strand_path>` and the system prompt via `--system-prompt`
- Tie-off files contain the agent's actual output
- Tests verify the CLI invocation is constructed correctly (adapter-focused, no real `pi` call needed)

## Implementation Status: ✅ Complete (2026-06-04)

## Hex Layer: Domain → Application → Outbound Adapters

Work flows inward-out:
1. **Domain** — extend `AgentConfig` with provider, model, tools; extend `WorkspaceAgentConfig` with pi-specific fields
2. **Domain** — update knot file parser to read new fields
3. **Application** — `ProcessStrand` builds CLI invocation from knot config (new helper function)
4. **Outbound adapters** — subprocess runner passes prompt/strand data to the CLI
5. **Integration** — end-to-end test with `pi -p --print` or a stub

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `domain::value_objects::tests` | AgentConfig, PromptTemplate, WorkspaceAgentConfig validation + serialization | ✅ Green — only `goal` field |
| `domain::knot_file::tests` | Knot file parsing (name, goal, input-bundling, instructions) | ✅ Green — no provider/model/tools |
| `application::usecases::tests` | ProcessStrand with mock AgentRunner — state transitions, error handling | ✅ Green — mock ignores CLI args |
| `adapters::subprocess::tests` | SubprocessAgentRunner spawn/timeout/error handling | ✅ Green — uses `echo`/`sh -c` |
| `tests/integration.rs` | Full pipeline with `sh -c "echo processed"` mock agent | ✅ Green — verifies tie-off content |
| `tests/http_interface.rs` | Health, agent listing | ✅ Green — baseline |
| `tests/filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline |

## Test Gaps

- No test verifies that `ProcessStrand` constructs CLI args from knot config
- No test verifies that the subprocess runner passes strand content to the agent
- No test verifies knot file parsing of `agent-config.provider`, `agent-config.model`, `agent-config.tools`
- No test verifies that `AgentConfig` validates new fields

## Phases

### Phase 0: Extend AgentConfig Domain Model
**Failing tests created:** `domain::value_objects::tests::agent_config_with_provider_and_model`, `domain::knot_file::tests::knot_file_with_provider_model_tools`

- [x] Add `provider`, `model`, `tools` (optional `Vec<String>`) fields to `AgentConfig`
- [x] Update `AgentConfig::new()` to accept and validate new fields (provider and model required, tools optional)
- [x] Update `Knot::new()` / construction to carry new fields
- [x] Update `KnotFile` and `RawAgentConfig` YAML parsing to read `provider`, `model`, `tools` from frontmatter
- [x] Update existing knot file fixtures in tests to include provider/model
- [x] Add `AgentConfig::build_cli_args()` helper — constructs `Vec<String>` of pi CLI flags from the config + prompt template

### Phase 1: ProcessStrand Builds CLI Invocation
**Failing tests created:** `application::usecases::tests::process_strand_builds_pi_cli_args`, `application::usecases::tests::process_strand_passes_prompt_and_strand_to_context`

- [x] `ProcessStrand::execute()` builds CLI args from `knot.agent_config.build_cli_args(knot.prompt_template)` instead of using raw `workspace_config.cli_args`
- [x] `ExecutionContext` carries the strand content (read from file) so the runner can pass it
- [x] CLI invocation pattern: `pi -p --model <model> --system-prompt "<instructions>" --no-session --no-tools "@<strand_path>"`
- [x] Update use case tests to verify constructed args via mock AgentRunner that records ExecutionContext

### Phase 2: Subprocess Runner Passes Prompt to Agent
**Failing tests created:** `adapters::subprocess::tests::runner_passes_prompt_via_stdin`, `adapters::subprocess::tests::runner_passes_strand_via_at_syntax`

- [x] SubprocessAgentRunner passes `cli_args` as constructed (now includes `@<strand_path>`)
- [x] SubprocessAgentRunner passes the prompt via stdin (pipe `ctx.prompt` to child's stdin)
- [x] Test with `sh -c "cat"` to verify stdin is received
- [x] Test with `cat /dev/stdin` to verify content round-trips
- [x] Keep existing tests green (they use `echo` which doesn't read stdin — should still work)

### Phase 3: Integration Test with Real pi CLI (or Stub)
**Failing tests created:** `integration::tests::full_pipeline_with_pi_agent`, `integration::tests::pi_agent_receives_system_prompt_and_strand`

Two approaches — pick one:

**Option A: Real `pi -p` call (if `pi` is available on CI/dev machine)**
- [ ] Integration test: start Knot with `WorkspaceAgentConfig { cli_path: "pi", ... }`
- [ ] Create knot with provider/model in config
- [ ] Create strand → verify tie-off contains LLM-generated content
- [ ] Requires API key for the chosen provider

**Option B: Stub script that mimics `pi -p`**
- [x] Create a shell script `stub-pi.sh` that reads `--system-prompt` and `@<file>` args and echoes them back
- [x] Integration test uses `stub-pi.sh` as `cli_path`
- [x] Verifies the invocation pattern (args received) without needing a real LLM
- [x] Lighter weight — no API key needed

- [x] Verify the full happy path: strand created → pi invoked with correct args → tie-off contains agent output
- [x] Verify the error path: nonexistent model → pi exits non-zero → knot-state shows `failed` with error

### Phase 4: Demo Verification
- [x] Update `knot-test` demo loom config with provider/model fields
- [x] Verify Knot processes `sample-document.md` and produces a populated tie-off
- [x] Verify loom-log records successful processing

## Notes

- The `pi` CLI supports `@<file>` syntax to include file content in the prompt. This is the natural way to pass strand content — Knot passes `@<strand_path>` as a positional arg to `pi`.
- The `--print, -p` flag makes `pi` run non-interactively: process the prompt and exit.
- `--no-session` prevents `pi` from persisting session files (Knot's tie-off is the output, not the session).
- `--no-tools` disables built-in tools (read, bash, edit, write) since Knot's agent runs in a read-only summarisation context. If future knots need tools, this can be configured via `agent-config.tools`.
- The system prompt comes from `prompt-template.instructions` in the knot file. The strand content comes from the watched file. Together they form the complete agent input.
