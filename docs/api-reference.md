# API Reference

Knot exposes a REST API on `localhost:3000` (default). An interactive
Swagger UI is available at `http://localhost:3000/swagger-ui`.

All endpoints use JSON for request and response bodies unless noted
otherwise.

## Health and System

### GET /health

Verify Knot is running.

```bash
curl http://localhost:3000/health
```

Response: `200 OK` with plain text body `ok`.

### GET /agents/{dir}

List agent names in a directory.

```bash
curl http://localhost:3000/agents/path/to/agents
```

Response: `200 OK` with `Array<String>`.

### GET /config/rig

Get the loaded rig configuration.

```bash
curl http://localhost:3000/config/rig
```

Response:

```json
{
  "rig_path": "/absolute/path/to/rig",
  "cli_path": "pi",
  "cli_args": []
}
```

### POST /config/reload

Re-scan the rig and register any looms not already in the store. Useful
for manual recovery when the file watcher misses an event.

```bash
curl -X POST http://localhost:3000/config/reload
```

Response: `200 OK` with `Array<LoomSummary>` of newly discovered looms.

## Looms

### GET /looms

List all discovered looms.

```bash
curl http://localhost:3000/looms
```

Response:

```json
[
  {
    "id": {"0": "prd-review-loom"},
    "knot_count": 2
  }
]
```

### GET /looms/{id}

Get loom details including all knot definitions.

```bash
curl http://localhost:3000/looms/prd-review-loom
```

Response:

```json
{
  "id": {"0": "prd-review-loom"},
  "knots": [
    {
      "id": {"0": "goals-review"},
      "agent_profile_ref": "fast",
      "prompt_template": {
        "instructions": "Review the goals section."
      },
      "strand_dir": "/absolute/path/to/project/prds"
    }
  ]
}
```

### GET /looms/{id}/knots

List knot names in a loom.

```bash
curl http://localhost:3000/looms/prd-review-loom/knots
```

Response: `["goals-review", "non-goals-review"]`

### GET /looms/{id}/activity

Get the loom's activity log.

```bash
curl http://localhost:3000/looms/prd-review-loom/activity
```

Response — array of event objects. Event types:

| Type | Meaning |
|------|---------|
| `LoomStarted` | Loom began processing |
| `LoomStopped` | Loom stopped processing |
| `KnotRegistered` | A knot was registered |
| `KnotDeregistered` | A knot was removed |
| `KnotParseWarning` | Unknown YAML property in knot file |
| `KnotProcessing` | Knot started processing a strand |
| `KnotCompleted` | Knot finished successfully |
| `KnotFailed` | Knot failed with an error |
| `StrandProcessed` | Strand was processed (success or failure) |

Example response:

```json
[
  {
    "LoomStarted": {
      "loom_id": {"0": "prd-review-loom"},
      "timestamp": "2026-06-10T12:00:00Z"
    }
  },
  {
    "KnotProcessing": {
      "loom_id": {"0": "prd-review-loom"},
      "knot_id": {"0": "goals-review"},
      "strand_path": {"0": "project/prds/goals.md"},
      "timestamp": "2026-06-10T12:00:02Z"
    }
  },
  {
    "KnotCompleted": {
      "loom_id": {"0": "prd-review-loom"},
      "knot_id": {"0": "goals-review"},
      "strand_path": {"0": "project/prds/goals.md"},
      "tie_off_path": {"0": "rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md"},
      "timestamp": "2026-06-10T12:00:03Z"
    }
  }
]
```

### GET /looms/{id}/knots/{name}

Get the processing status of a specific knot.

```bash
curl http://localhost:3000/looms/prd-review-loom/knots/goals-review
```

Response:

```json
{
  "knot_id": {"0": "goals-review"},
  "loom_id": {"0": "prd-review-loom"},
  "status": "completed",
  "last_strand_path": {"0": "project/prds/goals.md"},
  "last_tie_off_path": {"0": "rig/tie-offs/prd-review-loom/goals-review/goals-review-tie-off.md"},
  "last_error": null
}
```

**Status values:**

| Status | Meaning |
|--------|---------|
| `idle` | Knot registered but not yet processing |
| `processing` | Currently processing a strand |
| `completed` | Processing finished successfully |
| `failed` | Processing failed with an error |

## Profiles

### GET /profiles

List all agent profiles.

```bash
curl http://localhost:3000/profiles
```

Response:

```json
[
  {
    "name": "fast",
    "provider": "openai",
    "model": "gpt-4o",
    "tools": ["fs"],
    "system_prompt": "You are a fast reviewer."
  }
]
```

> **Note:** The `timeout` field is not included in API responses. To
> check a profile's timeout, read the file at
> `rig/profiles/{name}.md` directly.

### GET /profiles/{name}

Get a specific profile by name.

```bash
curl http://localhost:3000/profiles/fast
```

Response:

```json
{
  "name": "fast",
  "provider": "openai",
  "model": "gpt-4o",
  "tools": ["fs"],
  "system_prompt": "You are a fast reviewer. Keep responses concise and direct."
}
```

## Swagger UI

Full interactive API documentation with the ability to try requests
directly in your browser:

```
http://localhost:3000/swagger-ui
```

The OpenAPI spec JSON is available at:

```
http://localhost:3000/swagger-ui/openapi.json
```
