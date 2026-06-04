---
name: review-knot
agent-config:
  goal: "Review and summarize documents"
  provider: "openai"
  model: "gpt-4o"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the provided document. Provide a concise summary
    of its key points and any recommendations.
---

# Review Knot

This knot reviews and summarizes documents dropped into the
loom's source directory.
