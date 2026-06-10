---
name: review-knot
agent-config:
  goal: "Review documents"
  provider: "openai"
  model: "gpt-4o"
strand-dir: "src/workflow"
prompt-template:
  input-bundling: "full-file"
  instructions: "Review docs"
---

# review-knot
