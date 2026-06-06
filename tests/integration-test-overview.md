# Integration Test Overview

This directory contains Knot's integration test suite, organised by feature area. Each module spins up the real server (or exercises real components) and verifies end-to-end behaviour.

## Test Modules

| Module | Tests | Scope |
|--------|-------|-------|
| [`rig_lifecycle.rs`](rig_lifecycle.rs) | 5 | Rig directory auto-creation, loom scanning on startup, health/config endpoints, and registration persistence across restarts |
| [`composition.rs`](composition.rs) | 1 | Composition root wiring — verifies all hexagonal layers are connected correctly (no HTTP server) |
| [`discovery.rs`](discovery.rs) | 4 | Loom discovery from rig directory, filtering of non-loom directories, watcher boot, and `.loom-log` registration entries |
| [`pipeline.rs`](pipeline.rs) | 5 | Full event pipeline: Notify → debounce → ProcessStrand → tie-off. Covers strand lifecycle (create/modify/delete), HTTP observability, subdirectory rigs, and external source dirs |
| [`agent_integration.rs`](agent_integration.rs) | 3 | External agent CLI invocation — stub `pi` CLI happy path, agent error capture in knot-state and loom-log |
| [`tie_off.rs`](tie_off.rs) | 2 | Tie-off append-mode history and markdown section structure parsing |
| [`loom_crud.rs`](loom_crud.rs) | 3 | HTTP loom CRUD — register, discover, and unregister via REST, followed by strand processing |
| [`shutdown.rs`](shutdown.rs) | 2 | Graceful shutdown — watcher cessation and `.loom-log` `LoomStopped` event on signal |
| [`multi_loom.rs`](multi_loom.rs) | 2 | Multi-loom isolation (no cross-contamination) and per-knot source directory separation within a single loom |
| [`demo.rs`](demo.rs) | 2 | `knot-test` demo loom — provider/model fields, tools config, and tie-off generation with stub-pi |

## Pre-existing Modules (unchanged by refactor)

| Module | Tests | Scope |
|--------|-------|-------|
| [`http_interface.rs`](http_interface.rs) | 3 | HTTP handler unit tests (health, list agents) using `axum` Router — no server spawned |
| [`filesystem_interface.rs`](filesystem_interface.rs) | 3 | Filesystem create/list/roundtrip — minimal, focused |
| [`swagger_ui.rs`](swagger_ui.rs) | 2 | Swagger UI HTML and OpenAPI JSON spec served on mock ports |
| [`skill_integration.rs`](skill_integration.rs) | 19 | Skill file frontmatter validation, API contract tests against live endpoints |

## Shared Infrastructure

| Module | Purpose |
|--------|---------|
| [`helpers.rs`](helpers.rs) | Shared test fixtures: knot YAML creation, mock/stub agents, raw TCP HTTP helpers (`GET`, `POST`, `DELETE`), server spawning, port readiness, and knot-state polling |

## Notes

- Tests that spin up the server use raw TCP HTTP helpers (no `reqwest` dependency) to keep the test surface minimal.
- All integration tests run in parallel by default via `cargo test` — each test creates its own temp directory and binds to a random port.
- The suite was refactored from a single 3272-line `integration.rs` into the modules above (plan: `project/plans/integration-test-refactor.md`).
