# DPR-001: Multi-Knot Watch Fanout

**Created:** 2026-06-14
**Related Plan:** [Outbound Adapters (file-watcher.md)](../plans/file-watcher.md)

---

## Problem

When multiple knots watch the same strand directory, each knot should receive independent events for every file change. The naive implementation maps each watched directory to a single `(loom_id, knot_id)` pair — so when two knots register the same directory, only the last-registered knot receives events.

## How It Works

The fix is three-tier, each layer handling multi-knot independently:

### Tier 1: Watch Registration (`NotifyEventSource::register_watch`)

The watched directory list allows multiple entries per path, keyed by the full `(path, watch_type)` tuple:

```rust
// Before: path-only dedup — second knot overwrites first
if let Some(pos) = watched_dirs.iter().position(|(p, _)| p == &path) {
    watched_dirs[pos].1 = watch_type;  // ← overwrites!
}

// After: (path, watch_type) dedup — same knot is idempotent, different knots are appended
if let Some(pos) = watched_dirs.iter().position(|(p, wt)| p == &path && watch_types_equal(wt, &watch_type)) {
    watched_dirs[pos] = (path, watch_type);
} else {
    watched_dirs.push((path, watch_type));
}
```

`watch_types_equal` considers two `Strand` watches identical only when both loom and knot IDs match.

### Tier 2: Event Fanout (`NotifyEventSource::map_event`)

`find_watch_types` returns **all** `WatchType` entries at the most specific path depth (shadowing: a watch on `/a/b/` shadows `/a/`). `map_event` iterates all matching watches and emits one `StrandEvent` per `Strand` watch:

```rust
fn map_event(&self, event: &Event) -> (Vec<StrandEvent>, Option<ConfigEvent>) {
    let watch_types = self.find_watch_types(path);
    let mut strand_events = Vec::new();

    for wt in watch_types {
        if let WatchType::Strand(loom_id, knot_id) = &wt {
            if let Some(se) = self.map_strand_event(event, path, loom_id, knot_id) {
                strand_events.push(se);
            }
        }
    }
    (strand_events, config_event)
}
```

### Tier 3: Debounce (`DebounceEngine`)

The debounce engine keys on `(StrandPath, LoomId, KnotId)` — not just `StrandPath` — so rapid events for the same file but different knots are debounced independently:

```rust
type EventKey = (StrandPath, LoomId, KnotId);
let mut pending: HashMap<EventKey, (StrandEvent, Instant)> = HashMap::new();
```

## Usage Example

**Context:** Two knots in the same loom watch `project/prds/`:

```yaml
# planning-loom/foundational-arch.md
name: foundational-arch
agent-profile-ref: fast
strand-dir: "../project/prds"
...

# planning-loom/sharing-model.md
name: sharing-model
agent-profile-ref: fast
strand-dir: "../project/prds"
...
```

**Result:** When `project/prds/prd-sharing-is-caring.md` is modified, both knots receive independent debounced `StrandEvent::Modified` events and each processes the file through its own agent pipeline.

## Gotchas

- **Shadowing:** If a watch exists on `/a/b/` (strands) and `/a/` (rig), file events in `/a/b/` match both. The system picks the most specific depth — so rig-level config watches don't accidentally emit as strand events. This is handled in `find_watch_types` by computing max depth first.
- **Config watches are single:** `WatchType::Rig` and `WatchType::Loom` watches remain unique per directory (only one rig watch, one loom watch per directory). Only `WatchType::Strand` supports fanout.
- **Notify fires multiple raw events:** `notify` may emit 2-3 raw events per file modification (e.g., `Modify(MetadataWriteThenData)`). Each raw event triggers the full fanout. The debounce engine collapses these per `(file, knot)` within the 100ms window.
