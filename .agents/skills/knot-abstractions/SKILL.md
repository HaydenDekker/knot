---
name: knot-abstractions
description: "Understand the layered architecture of the Knot agent orchestration system and how it relates to project application layers. Covers the abstraction boundary between the orchestration engine (Knot), agent identities (profiles), reusable behaviors (skills), and the project domain layer. USE FOR: understand rig architecture, learn knot abstractions, grasp system layers, understand profile-skill relationship, learn how project folder feeds into system. DO NOT USE FOR: creating looms (use knot-create), inspecting state (use knot-inspect), designing knots (use knot-design)."
license: MIT
metadata:
  author: Knot Team
  version: "1.0.0"
  compatibility: "Knot 0.26.0+"
---

# Knot Abstractions Skill

This skill documents the layered architecture of the Knot agent
orchestration system and how it relates to the project application.
Read this before working deeply with any other knot skills.

## System Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│  Knot RIG (borrow-my-stuff-rig/) — Orchestration Engine │
│  • Declarative file-based graph                        │
│  • Single-threaded durable event queue                 │
│  • Nodes: Looms + Knots (edges)                         │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│  Profiles (rig/profiles/*.md) — Agent Identities         │
│  • Declare a role and set of skills                      │
│  • Reference a model/provider configuration              │
│  • Do NOT know about Knot — only their workflow          │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│  Skills — Generic, Reusable Behaviors                    │
│  • Project skills: domain-specific workflows             │
│  • Technical skills: technology-specific operations      │
│  • Cross-project, cross-orchestrator reusable            │
│  • Do NOT reference Knot, the rig, or specific workflows │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│  Application Layer (project/) — What We Build            │
│  • Domain glossary, plans, PRDs, acceptance specs        │
│  • ADRs, design documents, configuration management      │
│  • Pure domain/application knowledge                     │
└─────────────────────────────────────────────────────────┘
```

## The Four Abstraction Layers

### 1. The Rig (Orchestration Engine)

Located at `borrow-my-stuff-rig/`, the rig is the "dumb" plumbing that:
- Maintains a **declarative graph** defined in `.md` files:
  - **Looms** (directories ending in `-loom/`) group knots by domain
  - **Knots** (`.md` files inside looms) define workflow nodes
- Provides a **single-threaded durable event queue** that processes
  one event at a time, ensuring consistency
- Routes **events** between knots without the agents knowing it exists
- Maintains **state.json**, tie-offs, and logs

The rig is generic — it contains no project-specific logic.

### 2. Profiles (Agent Identities)

Located at `borrow-my-stuff-rig/profiles/*.md`, profiles declare:
- **Model/provider configuration** (which LLM to use)
- **Timeout settings** and available tools
- **A role** the agent session plays
- **Skills** available to the agent

Key principle: **Profiles don't know about Knot.** A profile like
`coding.md` says "I am a phase implementer with tools X, Y, Z" but
never references looms, strands, or events. This makes profiles
portable — the same profile could run under a different orchestrator.

### 3. Skills (Generic Behaviors)

Skills are reusable capabilities that can be composed into profiles:
- **Project skills** — domain-specific workflows (e.g., `project-acceptance`
  for BDD spec creation, `project-plan-completion` for plan finalisation)
- **Technical skills** — technology-specific operations (e.g., `playwright`
  for e2e tests, `tauri-system-architecture` for Tauri patterns)
- **System skills** — rig-wide operations (e.g., `knot-design`,
  `knot-inspect`, `knot-analyst`)

Key principle: **Skills are abstracted from orchestration.** A skill
like `project-acceptance` doesn't reference Knot, events, or the rig
directory structure. It describes a generic behavior ("create BDD specs
from PRDs") that could be invoked in any context — by Knot, by a
human, or by another orchestrator.

### 4. Application Layer (Project Domain)

Located at `project/`, this layer contains all domain-specific knowledge:
- **Domain glossary** — defines domain terms (e.g., "Steward," "Maker,"
  "Manifest" in this project)
- **Plans** (`project/plans/`) — feature implementation roadmaps
- **PRDs** (`project/prds/`) — product requirement documents
- **Acceptance specs** (`project/acceptance/`) — BDD scenarios
- **ADRs** (`project/adrs/`) — architectural decision records
- **Design docs** (`project/docs/`) — implementation knowledge
- **Configuration management** (`project/cm/`) — build outputs tracking

Key principle: **Profiles and skills cannot modify this layer.** The
`coding` profile explicitly states: "You must never modify, add or
delete a file contained in `project/` and its subdirectories." This
boundary ensures agents consume project knowledge without corrupting it.

## How the Layers Connect at Runtime

1. A **knot** (e.g., `phase-implementer.md` in `coding-implementation-loom/`)
   fires when its trigger event occurs.

2. The knot activates a **profile** (`coding.md`) with an event payload
   containing a reference to a specific plan file in `project/plans/`.

3. The profile/agent reads the phase checklist from that plan file and
   uses its **skills** (file operations, TDD practices, ADR references)
   to implement the phase.

4. Upon completion, the agent's output is captured as a **tie-off file**,
   and the rig may emit events to trigger downstream knots.

5. If more phases remain, the next `PhaseReady` event chains to the
   same or another knot/profile combination.

## Cross-Reference

This skill provides foundational context for:

1. **knot-design skill** — design looms and knots using these abstractions
2. **knot-create skill** — create knots that respect layer boundaries
3. **knot-inspect skill** — inspect rig state through this lens
4. **knot-analyst skill** — analyse rig productivity with architectural awareness
5. **knot-dispatch skill** — trigger knots understanding the layer connections

Related project skills:

1. **project-management skill** — overall project structure (includes this layer model)
2. **project-planner-structure skill** — planning standards that reference project documents
