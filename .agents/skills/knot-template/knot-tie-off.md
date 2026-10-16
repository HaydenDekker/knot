## Tie-Off Contract
You have a number of subscribers that have requested to be notified if certain events occur during this session. You
must acknowledge each event in your tie-off (final response). Subscribers can't begin their work until your events are delivered to
them via your tie-off.

Your final response **is** your tie-off: tie-offs are managed by the infrastructure service, so never write or read directly from `tie-offs/` files — deliver your output as your final response and emit event blocks there.

## Event Format

Emit one ```markdown block **per subscriber event** listed above.
Each block must have `---` frontmatter delimiters:

```markdown
---
event: EventId
occurred: true
description: Short summary of what happened
timestamp: 2026-08-06T14:30:00
<optional fields if specified in the event description above>
---

Freeform narrative context about the event.
```

## Rules

- Emit exactly one event block per subscriber event listed above.
- The `event` and `occurred` fields are required in every block.
- When `occurred: false`, the event is not dispatched but still
  counts as acknowledgement.
- Mentioning an event in narrative text is not an acknowledgement —
  only structured ```markdown event blocks are parsed. Emit a block
  for every event listed above, even when `occurred: false`.
- If a pending event satisfies the event that has just occurred, set `occurred: false` to avoid duplicated events.
- You may conclude a pending event is already relevant but requires additional context, but do not edit the pending event and instead
  emit a new event of the same type with additional context.
