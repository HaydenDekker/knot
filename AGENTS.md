# Knot — Agent Developer Notes

## What is Knot?

Knot is a **Rust** application that runs as a **local service** on a developer's machine. It orchestrates AI agent workflows, manages file-based configurations, and exposes an **HTTP control and observability interface** for interaction and monitoring.

## Architecture

- **Local-first** — Designed to run on a single developer workstation, not as a distributed cloud service.
- **File system access** — Knot reads and writes project files directly to manage agent profiles, prompt templates, and workflow state.
- **HTTP interface** — Provides RESTful endpoints for controlling agents, submitting workflows, and observing runtime state.

## Building

```bash
cargo build
```

## Running

```bash
cargo run
```

This starts the Knot HTTP service on `localhost:3000` (or the configured port).

## Domain Glossary

Domain terms used throughout the project are defined in [project/domain-glossary.md](project/domain-glossary.md). Read it before starting work on any feature.

> **Agent instruction:** When `project/domain-glossary.md` is updated, also update this section of AGENTS.md to keep them in sync. The authoritative source is `project/domain-glossary.md`.
