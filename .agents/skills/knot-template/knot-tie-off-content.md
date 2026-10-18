## Tie-Off Content

Your tie-off (final response) begins with the content of the work done in this
session — a short, factual summary: what was done or found, the key decisions
and their rationale, and pointers to the files changed or produced. Keep it
thin — facts and pointers, not a re-telling of the session.

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
- **Deduplication** — if a queued event (see the *Queued Events* section of your
  prompt, when present) already carries the same facts — compare the frontmatter
  keys such as plan/phase — mark the subscriber event `occurred: false` and say so
  in its `description` (it is already queued); do not queue a second signal for the
  same outcome.
