# Plan: Explicit Pi Session Title via `--name`

## Problem

After [simplify-agent-invocation](simplify-agent-invocation.md) (Plan #32), the profile prompt is prepended to stdin before knot instructions. Pi derives its session display name (the "resume title") from the first text it receives. Since the profile prompt is static per profile (e.g. "You are a fast reviewer"), every session gets the same resume title regardless of which strand or knot was processed.

## Target

Every pi session gets a unique, descriptive title passed via `--name` CLI flag, matching the trigger line format. The resume title will look like:

```
plan-architect triggered by Modified on 004-manifest-resources.md
```

Prompt content ordering is unchanged (profile prompt → knot instructions → trigger line).

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `subprocess.rs` unit tests | `build_prompt_with_context` ordering, CLI arg passthrough via `cat` | ✅ Green |
| `agent_integration.rs` | Full knot run with profile prompt | ✅ Green — verifies output content, not title |
| `value_objects.rs` — `agent_config_build_cli_args_basic` | `build_cli_args()` produces `["-p", "--model", "..."]` | ✅ Green |

## Test Gaps

- No test verifies the `--name` flag is present in CLI args
- No test verifies the title format includes knot name and strand filename
- No test verifies the title is unique per strand

## Phases

### Phase 1: Add `--name` to CLI args in ProcessStrand
- [ ] In `usecases.rs`, after building `cli_args` and before constructing `ExecutionContext`, append `--name` with a title matching the trigger line format
- [ ] Title format: `{knot-id} triggered by {event-type} on {strand-filename}` (e.g. `plan-architect triggered by Modified on 004-manifest-resources.md`)
- [ ] Edge case: if strand has no file name (shouldn't happen, but guard with `unwrap_or_default`)

### Phase 2: Tests
- [ ] Unit test in `subprocess.rs` — verify CLI args contain `--name` flag when passed through (use `sh -c 'cat >/dev/null; echo "$@"'` or inspect args)
- [ ] Unit test in `usecases.rs` — verify the title format matches trigger line: `{knot-id} triggered by {event-type} on {strand-filename}`
- [ ] Existing `runner_passes_prompt_via_stdin` test remains green (prompt content unchanged)

## Notes

- Pi's `--name, -n <name>` flag sets the session display name explicitly, bypassing auto-derivation from stdin content.
- This is a single-line change in `usecases.rs` plus tests. No changes needed to domain, ports, or adapters.
