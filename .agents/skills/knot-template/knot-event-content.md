## Event Content Guide
How to fill in each event block:

- `description` — explain **why** the event was (or wasn't) triggered, plus any additional
  requested information. One or two focused lines: the facts a consumer needs to act,
  not a re-statement of the session.
- `timestamp` — required when `occurred: true` (ISO 8601, the moment the event occurred).
- **Additional fields** — when the event description above specifies extra fields (e.g.
  `next-task-context`, `plan-id`), include them in the frontmatter when `occurred: true`,
  in the shape the description asks for.
- **Body** — the text after the closing `---` is freeform narrative context about the event:
  pointers to files, decisions and their rationale, and anything the consumer needs beyond
  the frontmatter. Keep it thin — facts and pointers, not narrative.
- A block with `occurred: false` still needs a `description` saying why it did not occur;
  its body is optional.
