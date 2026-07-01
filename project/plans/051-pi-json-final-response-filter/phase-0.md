# Phase 0: Adapter — Filter Final Response by `stopReason`

**Plan:** [Filter Final Response in Pi JSON Adapter](../pi-json-final-response-filter.md)

## Checklist
- [x] In `src/adapters/pi_json.rs`, remove the `message_end` handler block from `parse_json_line()`
- [x] In `src/adapters/pi_json.rs`, add `stopReason` filter to the `agent_end` messages iteration — only extract text when `stopReason` is `"stop"` or `"length"`
- [x] Update test fixture in `test_json_runner_parses_session_id` — add `"stopReason":"stop"` to the assistant message in `agent_end`
- [x] Update test fixture in `test_json_runner_parses_token_usage` — add `"stopReason":"stop"` to the assistant message in `agent_end`
- [x] Update test fixture in `test_json_runner_parses_response_text` — add `"stopReason":"stop"` to the assistant message in `agent_end`
- [x] Update test fixture in `test_json_runner_parses_message_end_response` — changed assertion: `message_end` no longer extracts text (response is empty)
- [x] Add test `test_json_runner_excludes_tool_use_messages` — `agent_end` with two assistant messages (`toolUse` + `stop`), only final text in response
- [x] Add test `test_json_runner_includes_length_stop_reason` — `agent_end` with `stopReason: "length"`, response text is included
- [x] Add test `test_json_runner_excludes_error_stop_reason` — `agent_end` with `stopReason: "error"`, response text is excluded
- [x] Add test `test_json_runner_multiple_tool_use_then_stop` — `agent_end` with 3+ assistant messages (toolUse, toolUse, stop), only final message text in response
- [x] Compile and verify no errors
- [x] Run full test suite (`cargo test`)
- [x] Run clippy (`cargo clippy`)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
