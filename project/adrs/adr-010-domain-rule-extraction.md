# ADR-010: Domain Rule Extraction

**Date**: 2026-06-29
**Status**: Accepted

## Context

The `ProcessStrand` use case had grown to contain significant business rule logic alongside orchestration code:

- **Strand file validity** — text/binary detection, temp file filtering, missing file handling (~80 lines)
- **Deletion prompt composition** — reading tie-off history, formatting scoped history with deletion notice (~70 lines)
- **Tie-off outcome derivation** — 3-way branching on success/failure/timeout to determine what to write (~40 lines)
- **Agent config resolution** — mapping profile fields into agent configuration (~30 lines)

These are **domain rules** (what counts as a valid strand, how a knot represents its deletion, what outcomes mean) embedded in **application orchestration** (fan results to ports, coordinate I/O). The use case file `process_strand.rs` was ~2,000 lines (use case ~545 + tests ~1,450), making any change requiring a read of the full file expensive.

The question was: where do business rules live? In the use case that applies them, or in the domain entities they describe?

## Decision

Domain rules are extracted from application use cases into the domain layer. Business logic about **what entities mean** lives on the entities themselves. Business logic about **how entities compose** lives on value objects. The application layer coordinates — it calls domain methods and fans results to ports.

### Architecture Overview

Before (rules embedded in use case):

```
┌─────────────────────────────────┐
│  Application Layer              │
│  ProcessStrand::execute()       │
│  ├─ text file check (80 lines)  │
│  ├─ deletion prompt (70 lines)  │
│  ├─ outcome branching (40 lines)│
│  └─ config resolution (30 lines)│
└──────────┬──────────────────────┘
           │ calls
           ▼
┌─────────────────────────────────┐
│  Domain Layer                   │
│  Knot, StrandPath, AgentProfile │
│  (data + parsing only)          │
└─────────────────────────────────┘
```

After (rules on domain entities):

```
┌─────────────────────────────────┐
│  Application Layer              │
│  ProcessStrand::execute()       │
│  ├─ strand.should_process()     │
│  ├─ profile.resolve_for_knot()  │
│  ├─ knot.deleted_prompt()       │
│  └─ TieOffOutcome::derive()     │
│  (~200 lines, mostly orchestration)
└──────────┬──────────────────────┘
           │ calls domain methods
           ▼
┌─────────────────────────────────┐
│  Domain Layer                   │
│  StrandPath::should_process()   │
│  AgentProfile::resolve_for_knot()│
│  Knot::deleted_prompt()         │
│  TieOffOutcome::derive()        │
│  (~200 lines of rule logic)     │
└─────────────────────────────────┘
```

### Implications for Design

**Domain methods take port traits, not adapter types.** When a domain rule needs external information (e.g., "is this file a text file?"), the rule calls through a domain-level port trait (`StrandFileChecker`) rather than importing an adapter. This keeps the domain layer decoupled from infrastructure.

**Port traits live in the domain layer when called from domain methods.** The `StrandFileChecker` trait was placed in `src/domain/entities.rs` (not `src/application/ports.rs`) because `StrandPath::should_process()` calls it directly. Placing it in application/ports would create a circular dependency (domain → application → domain).

**Structured result types over side effects.** Instead of logging warnings and returning `bool`, `should_process()` returns `Result<StrandCheckResult, StrandCheckError>` where `StrandCheckResult` is an enum (`Proceed`, `SkipBinary`, `SkipTemp`, `SkipMissing`, `ProceedWithWarning`). This makes the domain rule testable without mocks of I/O.

**Outcome derivation as a pure function.** `TieOffOutcome::derive(result)` converts `Result<AgentOutput, PortError>` into a domain enum. The use case then calls accessor methods (`should_write_tie_off()`, `tie_off_status()`) instead of implementing branching logic inline.

### Testing Strategy

Domain rules are tested with **domain tests** — no port mocks, no application wiring. Each extracted rule has its own test module in the domain file:

- `StrandCheckResult` variants — 7 tests in `entities.rs` using `TestFileChecker` (in-memory, no file I/O)
- `Knot::deleted_prompt()` — 3 tests verifying prompt composition with various tie-off states
- `TieOffOutcome::derive()` — 6 tests covering success, failure, timeout paths
- `AgentProfile::resolve_for_knot()` — 4 tests verifying field mapping

Domain tests run in `cargo test --lib` alongside application tests. Application tests in `process_strand.rs` verify pipeline flow, not individual rules.

## Consequences

### Positive

- **Use cases are readable** — `ProcessStrand::execute()` is ~200 lines of orchestration instead of ~545 of mixed concerns
- **Domain rules are independently testable** — no port mocks needed; test the rule in isolation
- **Domain rules are discoverable** — find the rule on the entity it describes, not buried in a use case
- **Reduced duplication** — shared test fixtures extracted from 6+ test modules into `test_fixtures.rs`

### Negative

- **Domain layer grows** — `entities.rs` and `value_objects.rs` gain ~200 lines of rule logic + tests
- **Port trait placement is unconventional** — `StrandFileChecker` lives in domain, not application/ports. This is correct for this pattern but may surprise developers expecting all ports in one place
- **Domain method signatures expose port traits** — `should_process(&dyn StrandFileChecker)` makes the trait a public API surface

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Keep rules in use cases | Use cases become unreadable; rules are duplicated across use cases that share the same domain concept |
| Create separate `domain/rules/` module | Adds indirection. The rule belongs on the entity it describes (`StrandPath` knows what makes a strand valid) |
| Move all application logic to domain | Domain shouldn't know about I/O outcomes. `TieOffOutcome::derive()` is the boundary — it interprets results, not produce them |
| Use dependency injection for all adapters | Overhead for simple checks. Domain methods take `&dyn Trait` parameters, not stored fields (except where the use case already owns the adapter) |

## References

- Source: `src/domain/entities.rs` — `StrandCheckResult`, `TieOffOutcome`, `Knot::deleted_prompt()`, `StrandPath::should_process()`
- Source: `src/domain/value_objects.rs` — `AgentProfile::resolve_for_knot()`, `AgentProfile::session_timeout()`
- Source: `src/adapters/outbound/strand_file_checker.rs` — `ContentInspectorChecker` adapter
- Source: `src/application/usecases/process_strand.rs` — slimmed use case
- Related: [ADR-009: Agent-Specific Adapters](adr-009-agent-specific-adapters.md) — adapter specificity principle applies to `ContentInspectorChecker`
