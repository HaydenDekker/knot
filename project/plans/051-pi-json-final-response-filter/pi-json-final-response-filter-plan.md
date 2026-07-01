# Plan 51: Filter Final Response in Pi JSON Adapter

## Problem

The `PiJsonAgentRunner` (`src/adapters/pi_json.rs`) extracts response text from Pi's JSON-L output by concatenating text from **all** assistant messages. When the agent uses tools (file read, bash execution, etc.), multiple assistant messages are produced: intermediate ones that call tools (`stopReason: "toolUse"`) and the final response (`stopReason: "stop"` or `"length"`). Currently, **all** assistant message text is concatenated, so the tie-off file contains the full conversation including tool-call reasoning, not just the final answer.

Two handlers contribute to the problem:

1. **`message_end` handler** — extracts text from every `role: "assistant"` message (including intermediate tool-use messages)
2. **`agent_end` handler** — iterates the `messages` array and extracts text from every `role: "assistant"` message (including intermediate tool-use messages)

The `agent_end` handler duplicates the work of `message_end`, and neither filters by `stopReason`.

Pi's `AssistantMessage` type carries a `stopReason` field that distinguishes message types:

| `stopReason` | Meaning | Should include in response? |
|---|---|---|
| `"toolUse"` | Message ended because it called tools | **No** — intermediate |
| `"stop"` | Message completed normally | **Yes** — final response |
| `"length"` | Message truncated by max tokens | **Yes** — final response (truncated) |
| `"error"` / `"aborted"` | Error condition | No |

Intermediate messages (e.g. "Let me check the file...") have `stopReason: "toolUse"` — they called tools and are followed by tool results and more assistant turns. Only the final response has `stopReason: "stop"`.

## Target

The tie-off body contains only the agent's final response text, not intermediate tool-use correspondence. The `PiJsonAgentRunner` extracts response text using `stopReason` as the discriminator:

- `agent_end` handler: only extract text from assistant messages where `stopReason` is `"stop"` or `"length"`
- `message_end` handler: removed (its work is fully covered by the `agent_end` handler and it can't reliably filter — `message_end` events fire for all messages including intermediates)

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `src/adapters/pi_json.rs` — `test_json_runner_parses_session_id` | Session ID extraction from JSON-L | ✅ Green |
| `src/adapters/pi_json.rs` — `test_json_runner_parses_token_usage` | Token usage extraction from `agent_end` | ✅ Green |
| `src/adapters/pi_json.rs` — `test_json_runner_parses_response_text` | Response text from `agent_end` messages array | ✅ Green — single assistant message |
| `src/adapters/pi_json.rs` — `test_json_runner_parses_message_end_response` | Response text from `message_end` event | ✅ Green — single message |
| `src/adapters/pi_json.rs` — `test_json_runner_prompt_passthrough` | Full subprocess passthrough | ✅ Green |
| `src/adapters/pi_json.rs` — `test_json_runner_context_timeout_override` | Timeout enforcement | ✅ Green |

## Test Gaps

- No test with multiple assistant messages (tool-use cycle) — can't verify intermediate messages are excluded
- No test verifying `stopReason` filtering behaviour
- No test that `message_end` intermediate messages don't leak into response
- No test for `stopReason: "length"` (truncated) — should still be included

## Phases

### Phase 0: Adapter — Filter Final Response by `stopReason`

Work in `src/adapters/pi_json.rs` only.

**Changes:**

1. **Remove `message_end` handler** — its work is fully covered by `agent_end`. The `message_end` handler cannot distinguish intermediate from final messages in a multi-turn session, and `agent_end` has the complete message array. Removing `message_end` eliminates a source of duplication.

2. **Filter `agent_end` by `stopReason`** — only extract text from assistant messages where `stopReason` is `"stop"` or `"length"`:
   ```rust
   // In agent_end handler, inside the messages iteration:
   if role == "assistant" {
       let is_final = msg.get("stopReason")
           .and_then(|r| r.as_str())
           .map(|r| r == "stop" || r == "length")
           .unwrap_or(false);
       if is_final {
           // extract text content
       }
   }
   ```

3. **Update existing test fixtures** — all existing unit tests that construct JSON-L with `agent_end` must include `stopReason: "stop"` on assistant messages, otherwise the filter excludes them:
   - `test_json_runner_parses_session_id` — add `"stopReason":"stop"` to assistant message
   - `test_json_runner_parses_token_usage` — add `"stopReason":"stop"` to assistant message
   - `test_json_runner_parses_response_text` — add `"stopReason":"stop"` to assistant message

**New tests:**

- `test_json_runner_excludes_tool_use_messages` — `agent_end` with two assistant messages: one `stopReason: "toolUse"` and one `stopReason: "stop"`. Only the final message text appears in response.
- `test_json_runner_includes_length_stop_reason` — `agent_end` with `stopReason: "length"`. Response text is included (truncated responses are still valid final output).
- `test_json_runner_excludes_error_stop_reason` — `agent_end` with `stopReason: "error"`. Response text is excluded.
- `test_json_runner_multiple_tool_use_then_stop` — `agent_end` with 3+ assistant messages (toolUse, toolUse, stop). Only the final message text appears in response.

**Regression tests to verify:**
- All existing tests still pass after adding `stopReason` to fixtures
- `test_json_runner_prompt_passthrough` — subprocess mock echoes stdin (no JSON-L), unaffected
- `test_json_runner_malformed_json_fallback` — non-JSON input, unaffected
- `test_json_runner_empty_output` — empty input, unaffected

## Notes

- This is a **bugfix**, not a feature. The tie-off body should contain the final response, not the full conversation transcript.
- The change is backwards compatible with the `AgentOutput` type — `stdout` still contains the response text, just filtered correctly.
- `message_end` events are not needed for any other data extraction (session ID comes from `session` event, token usage from `agent_end`). Removing the handler is safe.
- `stopReason: "length"` is included because a truncated response is still the agent's final output (just incomplete). Excluding it would lose the response entirely for long outputs.
- If `agent_end` produces no messages with `stopReason: "stop"` or `"length"` (e.g. all tool calls errored), `response_text` will be empty. This is correct — no final response was produced. The caller will see an empty tie-off body.
