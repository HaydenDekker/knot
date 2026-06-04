# PRD: Knot Skills — AI-Driven Configuration via Skills

## Problem

Configuring Knot — creating rigs, defining looms and knots, inspecting their status — currently requires the user to manually create files, edit YAML/markdown, and call HTTP endpoints by hand. There is no guided, conversational way to set up Knot. A user with a clear goal in mind (e.g. "I want to review all PRDs in my project") must still understand the full Knot domain model, file layout, and API surface to achieve it. This friction makes Knot hard to adopt and slow to configure.

## Goals

- [ ] Provide AI skills that let a user configure Knot entirely through natural language — no manual file editing or API calls required
- [ ] Expose a machine-readable OpenAPI (Swagger) spec so skills can reference the exact API contract and validate their calls
- [ ] Cover the full configuration lifecycle: initialise a rig, create/modify/delete looms and knots, and inspect current state

## Non-Goals

- Running agent sessions or triggering knot processing — that is Knot's core runtime, not configuration
- Auth or multi-user support — Knot is local-first, single-user
- A visual or GUI configuration tool — the skills ARE the configuration interface

## User Stories

### Story 1: Initialise a Knot Rig

As a new Knot user, I want to run `knot init` in my project directory so that a rig folder is created (if not present), Knot is verified to be running, and if Knot is not on my global path it is built and installed before the rig is initialised.

**Scenarios:**
1. Given I am in a project directory with no rig folder, when I run `knot init`, then the rig folder is created relative to the current directory, Knot is checked to be running, and if it is not on the global path it is built and installed, after which I can confirm the setup succeeded
2. Given a rig folder already exists, when I run `knot init`, then the command confirms the rig is present and offers to show its current state instead of recreating it
3. Given Knot is not on the global path, when I run `knot init`, then Knot is built from source and installed locally so that the init flow can proceed
4. Given Knot is not running, when I run `knot init`, then the command detects this and either starts Knot or reports an error with guidance on how to start it

### Story 2: Configure Looms and Knots

As a user, I want to describe what I want to achieve (e.g. "review my PRDs", "summarise my changelog") so that looms and knots are created, modified, or deleted to match my intent.

**Scenarios:**
1. Given I have a rig, when I describe a new loom (source directory, target, knot definitions), then a skill calls Knot's HTTP API to create the loom and its knots and I receive confirmation
2. Given a loom exists, when I describe changes to it (e.g. change the target, add a new knot, update a prompt template), then a skill calls Knot's HTTP API to modify the loom and I receive confirmation
3. Given a loom exists, when I ask to delete it, then a skill calls Knot's HTTP API to remove the loom and I receive confirmation
4. Given I describe something ambiguous, then the skill asks clarifying questions before making API calls

### Story 3: Inspect Rig Status

As a user, I want to see the current state of my rig — which looms exist, what knots are configured, and their processing status — so I can understand what is set up and whether it is working correctly.

**Scenarios:**
1. Given a configured rig, when I ask to inspect my rig, then a skill calls Knot's HTTP API and presents a summary of looms, knots, and their state
2. Given I ask about a specific loom, then a skill calls Knot's HTTP API and shows me that loom's knots, target, and recent activity
3. Given I ask about a specific knot's state, then a skill calls Knot's HTTP API and shows me processing events, tie-offs, and any errors

### Story 4: Browse the API with Swagger

As a user or developer, I want to browse Knot's API in a web UI so I can inspect endpoints, test calls manually, and understand the contract.

**Scenarios:**
1. Given Knot is running, when I open the Swagger UI in my browser, then I see a browsable, interactive API documentation with all endpoints listed
2. Given a skill needs to verify an API contract, when it reads the OpenAPI spec, then it can validate request/response shapes against the documented schema

## Success Criteria

- [ ] Three skills exist: `knot-init`, `knots-and-looms`, `knot-inspect`, each callable by name and each using Knot's HTTP interface exclusively
- [ ] Swagger UI is served by Knot at a known path (e.g. `/swagger-ui`) and documents all public HTTP endpoints
- [ ] OpenAPI spec is generated from code (via `utoipa`) — not hand-maintained — so it stays in sync with the API
- [ ] A user can go from zero to a fully configured loom with knots using only natural language through the skills
- [ ] A user can inspect any aspect of their rig state through the `knot-inspect` skill

## Dependencies & Constraints

- Technical: Knot must expose HTTP endpoints covering rig init, loom CRUD, knot CRUD, and status inspection — these are prerequisites for the skills
- Technical: `utoipa` and `utoipa-swagger-ui` dependencies must be added to Knot for OpenAPI generation and Swagger UI serving
- Constraint: Skills interact with Knot only via its HTTP interface — no direct file system access by the skills (Knot manages its own files)
- Constraint: Knot is local-first — skills assume `localhost` and no authentication

## Implementation Status: 🟡 In Progress
