---
name: knot-software-factory
description: "Defines how to use Knot to build a software factory. The generic process (GProc) is the rig itself — the factory's workflow — and a factory rig should define a generic, technology-neutral process rather than a specific one; this skill is the blueprint for building such a rig. Covers the factory's layer model (GProc / GP / GA / SA) and the dependency rules between them, the interface abstractions (project/arch.md, project/sa.md) that bind the generic process to a chosen architecture and a specific application, the generic factory shape (concern looms, worker profiles, event backbone, completion record, orchestrator loop-break), and where technology knowledge lives so the process stays reusable across stacks and projects. USE FOR: software factory, factory rig, factory design, GProc, generic process, generic product, generic application, specific application, build a factory, factory layers, factory reusability, factory workflow. DO NOT USE FOR: writing loom/knot/profile files (use knot-create), starting or stopping the service (use knot-start), inspecting rig state (use knot-inspect), technology-stack content (use the product skills, e.g. architectures), knot-level idempotency and loop design (use knot-design)."
license: MIT
metadata:
  author: Knot Team
  version: "1.1.0"
  compatibility: "Knot 0.41.0+"
---

# Knot Software Factory Skill

The **generic process (GProc)** of a software factory is the **rig
itself** — the workflow of looms, knots, and profiles that takes a
scoped product from requirements to deployed, validated, documented
software. This skill is the **blueprint** for building such a rig and
the layer model that keeps the rig's process reusable across stacks and
projects. The skill is not the process; the rig is.

A Knot **rig is a workflow**, and any workflow can be a rig. The factory
is one such workflow; Knot does not privilege it. When you create a
software factory, the rig should define a **generic** process — not a
technology-specific one — so one rig serves any project that provides
the right interface documents.

Read this before designing or building a factory rig. For knot-level
design rules (idempotency, events, loops) read `knot-design`. For the
runtime layering (rig → profiles → skills → application) read
`knot-abstractions`.

## What layer Knot fills

Three things, three layers:

- **Knot (the engine)** fills the **process-execution layer**: it runs
  the process — the durable event queue, the declarative looms and
  knots, the agent invocations, the tie-off records. The engine knows
  nothing about your stack or your product by construction.
- **The rig is the process** (the GProc) — the workflow made concrete
  in looms, knots, and profile files. *This* is where genericity is won
  or lost: an engine-neutral runtime does not make a rig generic. The
  rig files must not encode — and must not need to know — **what stack
  the product is built on** (the product layer) or **what this
  particular project builds** (the application layer).
- **This skill** is the blueprint for building the process generically.

The engine executes whatever the rig files declare. This skill's job is
to make the rig files declare *only the process* — and to point at
everything else through a small, stable set of **interface documents**.

The factory metaphor: the rig is the factory; looms are its departments
(concerns); knots are the communication channels and tasks; profiles are
the workers; events are the conversations; tie-offs are the work records;
the project documents are the factory infrastructure the workers build
against.

## The layer model

Four content layers, plus the interfaces that bind them:

| Layer | Name | Answers | Lives in |
|-------|------|---------|----------|
| GProc | **Generic Process** | *How a factory builds software* — concerns, workers, channels, chains, loop-break | The factory rig itself — its loom, knot, and profile files (this skill is its blueprint) |
| GP | **Generic Product** | *What the stack is* — the technology architecture | External skills (e.g. `architectures`) |
| GA | **Generic Application** | *How to apply the product* — the patterns, conventions, and worked examples for using the stack | The same or sibling external skills |
| SA | **Specific Application** | *What this project builds* — scope, domain, features, on the stack | Project documents |
| — | **Interfaces** | *Which architecture and which SA this project binds to* | `project/arch.md`, `project/sa.md` |

Two orthogonality axes:

- **Process vs product** — GProc knows how the factory works; GP/GA know
  what the stack is and how to apply it. Neither carries the other's
  content.
- **Generic vs specific** — GP/GA are stack-generic; the SA is
  project-specific. A process improvement must not touch the SA; a
  product change must not touch GProc.

How this sits over the `knot-abstractions` layers: the GProc **is** the
rig layer (with its profiles); the skill layer carries this blueprint
and the GP/GA product skills; the application layer carries the SA and
the interfaces.

## Dependency rules — the reusability contract

```
                    GProc — the factory rig itself
                                    │
                                    │ reads (its only downward edge)
                                    ▼
                      ┌────────────────────────┐
                      │  Interfaces (ports)    │
                      │  project/arch.md       │
                      │  project/sa.md         │
                      └─────┬────────────┬─────┘
                  names the │            │ names the
              ┌─────────────┘            └─────────────┐
              ▼                                         ▼
      ┌──────────────┐                        ┌────────────────┐
      │   GP / GA    │ ◄─────── built on ───── │      SA        │
      │  (skills)    │                        │   (project)    │
      └──────────────┘                        └────────────────┘
```

1. **GProc → interfaces only.** Rig files (knots, profiles) name the
   interface documents and follow the *pointers* those documents carry
   into GP/GA skills or project documents. They never inline stack
   knowledge or product specifics.
2. **SA → GP/GA.** The SA is written in the stack's terms: what is
   built, and how the stack delivers it. It never says how the factory
   works.
3. **SA ↛ GProc.** The SA must make sense without the factory: the same
   application could be built by humans, or by another process, from the
   same documents.
4. **GP/GA ↛ GProc and GP/GA ↛ SA.** Product skills stay
   orchestrator-agnostic and project-agnostic (per the
   `knot-abstractions` layering).
5. **The interfaces are the only seam.** `project/arch.md` and
   `project/sa.md` are the *ports* the process compiles against. They
   are thin — pointers, not content — and their names are part of the
   GProc contract: stable across projects, so one factory rig can be
   instantiated against any project that provides them.

**Why this holds:** each layer has one reason to change. A stack change
touches GP/GA (and possibly the SA); a product change touches the SA and
the interfaces; a process improvement touches GProc. If a change has to
touch two layers for one reason, a dependency has leaked across the
interfaces.

## The interface documents

### `project/arch.md` — the architecture binding

Specifies *which architecture* the process works with:

- The **GP/GA skill reference(s)** — which product skill(s) the factory's
  workers use (e.g. the `architectures` skill and its Tauri / frontend /
  PWA references).
- A **pointer to the project's full system-architecture document** (the
  SAD), where one exists — referenced, never summarised.
- The **cross-cutting constraints** the process must respect (e.g. "the
  architecture is fixed; new decisions go through ADR review").

### `project/sa.md` — the specific application

Specifies *what this project builds*:

- The **product shape and scope**, in the stack's terms (GP/GA).
- The **domain vocabulary** the workers share.
- **Pointers to the requirements** (PRDs, acceptance specs) —
  referenced, never copied.

### The contract

- **Thin and pointer-heavy.** The interface documents name what to read;
  they do not duplicate what is read. A worker that needs stack detail
  follows the pointer into the GP/GA skill or the SAD.
- **Stable names, evolving content.** The names are the GProc contract;
  the content changes as the project evolves.
- **Process-neutral.** Every profile in the rig can read both
  documents; they contain no knot mechanics — no event names, no loom
  names, no strand references.

## The generic factory shape

The process, generalised from the reference instance. A factory
instantiates the concerns its SA needs — the list is a vocabulary, not a
mandate.

### Concerns as looms

| Concern | Loom | Question it owns |
|---------|------|------------------|
| Strategy | `strategy-loom` | What does the project build next? |
| Planning | `planning-loom` | How is the work phased, tracked, and finalised? |
| Authoring | `author-loom` | Who turns a scoped target into an implementation plan? |
| Architecture | `architecture-loom` | Does the work conform to the fixed architecture? |
| Solution design | `solution-design-loom` | Which config item carries which piece of which story? |
| Configuration | `config-loom` | What are the config items, and are the documents aligned with the architecture? |
| Implementation | `coding-implementation-loom` | How are the plan phases built as code? |
| Review | `review-loom` | Does the delivered code meet the quality criteria? |
| Test planning | `test-planning-loom` | Does every config item have an executable test plan? |
| Validation | `validation-loom` | Does the delivered work pass its acceptance tests? |
| Acceptance | `acceptance-loom` | Does every story have an acceptance spec? |
| Deployment | `deployment-loom` | Are the config items brought to their deployment modes? |
| Documentation | `documentation-loom` | Do the user documents reflect delivered capabilities? |
| Orchestration | `orchestrator-loom` | Is the idle queue done, stuck, or work-remaining? |

The knot-level design of each loom (naming, idempotency, event
contracts, loop-breaking) is `knot-design` territory; this skill fixes
only the concern set and the backbone.

### Profiles as workers

Profiles are the factory workforce, described by **role only** — never
by model or provider, which change regularly. Two stable patterns:

- **Specialised workers** — one per concern (implementer, planner,
  validator, deployer, …).
- **Shared thinking pair** — a reasoning-heavy *judge* and a mechanical
  *reader* shared across the read/review looms, split by reasoning depth
  rather than by loom.

A profile's role text names its GProc duties (from this skill) and the
GP/GA skill(s) its stack work needs (from `project/arch.md`) — it never
inlines stack content.

### The event backbone

Three structures carry the factory:

1. **The delivery chain** — forward progress through the concerns:
   requirements → acceptance specs; scope → plan authored → plan ready →
   phase implementation chain → progress report → review + user docs;
   plan finalised → plan-scoped validation + deployment + test-plan
   sync; terminal finalisation → full-surface validation sweep.
2. **The rectification loop** — validation failure → gap assessment →
   rectification scope → targeted fix → re-validation. Bounded: a gap
   already tracked by an open plan or issue record is not re-raised.
3. **The loop-break** — the orchestrator (below).

The event vocabulary (`PlanScoped`, `PlanAuthored`, `PlanReady`,
`ProgressReport`, `PlanComplete`, `ValidationSweep`, …) is **process
vocabulary**: it belongs to GProc. Events carry facts, not instructions,
per `knot-design`.

### The completion record

The process maintains its own durable "what is done" record — e.g. a
VCRM (validation/completion matrix over config items × acceptance
specs) plus the plan lifecycle statuses. This is the **done-oracle**:
the orchestrator judges "Done" from the record, not by inspecting the
product. The record is process-shaped (plans, CI×spec cells) — it tracks
completion, not product content.

### The orchestrator and the loop-break

Every factory needs a bounded reaction to an idle queue:

- The orchestrator loom strands on the rig-scoped `QueueIdle` system
  event (0.41.0+).
- On each idle it reviews the delivered work (via the `knot-manage`
  skill) and checks the completion record, then classifies:
  - **Done** — nothing remains → stop the service.
  - **Stuck** — an unchanged state fingerprint across repeats, past the
    no-progress bound → stop the service and report.
  - **Work remains** — a draft/active/blocked plan, or an incomplete
    record → re-kick the pipeline at the strategy decision point (it
    never decides *what* to build — that is strategy's job).
- Every decision is recorded to a durable decision log.

This is the factory's loop-break: the `QueueIdle` → dispatch →
`QueueIdle` cycle is guaranteed to converge, because both terminal
outcomes stop the service and the re-kick path is bounded by the
no-progress fingerprint.

## Where technology knowledge lives

| Knowledge | Lives in |
|-----------|----------|
| How the factory works — concerns, chains, loops, loop-break | GProc — the factory rig (documented by this blueprint) |
| What the stack is, and how to apply it | GP/GA — external skills (e.g. `architectures`) |
| What this project builds | SA — project documents, via `project/sa.md` |
| Which stack + which application this project binds | Interfaces — `project/arch.md`, `project/sa.md` |
| Worker roles, models, tools | Profile files in the rig (parameterised via the interfaces) |
| Task instructions on each channel | Knot files in the rig |

**The leak test:** grep the rig files for concrete technology names
(build tools, frameworks, deployment targets). Any hit outside a
pointer line into an interface document is a leak — move that knowledge
into the GP/GA skill or the interface documents.

### When a process is technology-specific

A process — the rig you build — *can* be stack-specific, but when
creating a software factory the advice is to keep the rig's process
generic. If the stack genuinely needs process extensions (e.g. a
stack-specific build-and-verify step):

1. **Prefer** — keep the rig's process generic; put the stack-specific
   step in a stack skill (GP/GA) that the relevant profile invokes.
2. **Only if** the *event flow itself* is stack-specific — create a
   *derivative blueprint* (e.g. `knot-<stack>-factory`) that references
   this one and extends the concern set; its rig is then an explicitly
   stack-specific process. Never fork this blueprint, and never inline
   stack knowledge into a generic rig.

## Building a factory for a project

1. **Interfaces first.** Write `project/arch.md` (which GP/GA skill,
   pointer to the full SAD, constraints) and `project/sa.md` (what is
   built, in the stack's terms, pointers to the requirements).
2. **Confirm the product skills exist.** Reuse an existing GP/GA skill
   (e.g. `architectures`) or create one — always *external* to this
   skill.
3. **Instantiate the rig** (`knot-init`), then create the factory's
   looms, knots, and profiles (`knot-create`) following the generic
   shape above. Profiles reference the knot skills, the `knot-manage`
   skill for the orchestrator's reviews, and the GP/GA skill(s) named
   in `project/arch.md`. These files *are* the GProc — write them
   generic.
4. **Wire the loop-break** — orchestrator loom on `QueueIdle`,
   completion record in place, no-progress bound set.
5. **Audit the boundary** — run the leak test; confirm the interface
   documents are the rig's only downward edge.
6. **Run and evolve** (`knot-start`, `knot-dispatch`, `knot-manage`).
   Route every change to its layer: process change → the rig (and this
   blueprint when the change belongs to the generic shape); stack
   change → GP/GA; product change → SA and the interfaces. A change
   that needs to touch two layers for one reason means the interfaces
   need work.

## Reference instance

The first concrete GProc is the `software-factory-rig` at
`/home/hayden/workspace/proto/apps/pwa-todo-3/software-factory-rig/` —
a generic-process rig ("main generic software factory for knot"): 14
concern looms, ~16 worker profiles, orchestrator with the `QueueIdle`
loop-break. The rig stays generic; the *project* it runs in supplies
the interfaces that bind it to a web/PWA stack. Its `rig-overview.md`
maps that process; this skill is the blueprint it was built from.

## Cross-reference

- **knot-abstractions** — the runtime layering this skill's content
  partition sits over.
- **knot-design** — the knot-level rules: idempotency, events as facts,
  naming, loop-breaking, multi-session stores.
- **knot-create / knot-init / knot-start / knot-dispatch /
  knot-inspect / knot-manage / knot-analyst** — operating the rig this
  skill designs.
- **Product skills (GP/GA)** — e.g. the `architectures` skill: the
  generic product and its generic application, external to this skill by
  contract.
