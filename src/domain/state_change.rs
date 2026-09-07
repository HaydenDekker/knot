//! State-change diff — plan 083 (consolidated service log).
//!
//! [`diff_state`] compares two [`RigState`] snapshots and produces a
//! structured [`StateChange`] describing exactly which aspects changed.
//! The result feeds the `[KNOT][STATE]` delta lines in the service log:
//! each aspect gets a short, greppable line (one per changed
//! loom/knot/profile/queue-entry), never a full JSON dump.
//!
//! Comparison keys:
//!
//! - looms by `id`
//! - knots by `id` within the parent loom (looms present in both
//!   snapshots only)
//! - profiles by `name`
//! - queue entries by the queue's own `dedup_key`
//!   `(strand_path, loom_id, knot_id, event_kind)`
//!
//! `updated_at` (the write tick) and `rig_path` (process-constant) are
//! excluded from the diff.

use crate::domain::entities::{RigState, RigStateStrandQueueEntry};
#[cfg(test)]
use crate::domain::value_objects::ThinkingLevel;

/// A single knot-field change: `(field, from, to)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnotField {
    /// `status` — `idle`/`processing`/`completed`/`failed`.
    Status,
    /// `last-strand-path` — the strand path the knot is running on.
    LastStrandPath,
    /// `last-tie-off-path` — the tie-off path written on the last run.
    LastTieOffPath,
    /// `last-error` — the error recorded on the last failed run.
    LastError,
    /// `last-event-at` — the timestamp of the last processed event.
    LastEventAt,
}

/// A single profile-field change: `(field, from, to)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileField {
    /// `model-ref` — alias in the model registry.
    ModelRef,
    /// `provider` — provider key in the model registry.
    Provider,
    /// `model` — model name.
    Model,
    /// `thinking-level` — the `pi --thinking` token.
    ThinkingLevel,
    /// `timeout` — agent timeout in seconds.
    Timeout,
}

/// Per-knot field changes inside a loom present in both snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnotUpdate {
    /// Loom ID.
    pub loom: String,
    /// Knot ID.
    pub knot: String,
    /// One entry per changed field: `(field, from, to)`.
    pub field_changes: Vec<(KnotField, Option<String>, Option<String>)>,
}

/// Per-profile field changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileUpdate {
    /// Profile name.
    pub name: String,
    /// One entry per changed field: `(field, from, to)`.
    pub field_changes: Vec<(ProfileField, Option<String>, Option<String>)>,
}

/// A structured description of what changed between two `RigState`
/// snapshots (or `None` for the first snapshot of a run).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateChange {
    /// Loom ids present in the new snapshot but not the previous one.
    pub looms_added: Vec<String>,
    /// Loom ids present in the previous snapshot but not the new one.
    pub looms_removed: Vec<String>,
    /// Knots added: `(loom, knot)` — only within looms present in both.
    pub knots_added: Vec<(String, String)>,
    /// Knots removed: `(loom, knot)` — only within looms present in both.
    pub knots_removed: Vec<(String, String)>,
    /// Field-level changes for knots present in both.
    pub knot_updates: Vec<KnotUpdate>,
    /// Profile names present in the new snapshot but not the previous.
    pub profiles_added: Vec<String>,
    /// Profile names present in the previous snapshot but not the new.
    pub profiles_removed: Vec<String>,
    /// Field-level changes for profiles present in both.
    pub profile_updates: Vec<ProfileUpdate>,
    /// Queue entries added (keyed by the queue's dedup key).
    pub queue_added: Vec<RigStateStrandQueueEntry>,
    /// Queue entries removed (keyed by the queue's dedup key).
    pub queue_removed: Vec<RigStateStrandQueueEntry>,
}

/// Compare two `RigState` snapshots and return the structured change.
///
/// `prev = None` means this is the first snapshot of the run; the
/// result is the empty change (all of the new snapshot is "baseline",
/// not a change).
pub fn diff_state(prev: Option<&RigState>, new: &RigState) -> StateChange {
    let Some(prev) = prev else {
        return StateChange::default();
    };

    let mut change = StateChange::default();

    // ── Looms (by id) ──
    let prev_loom_ids: Vec<&str> = prev.looms.iter().map(|l| l.id.as_str()).collect();
    let new_loom_ids: Vec<&str> = new.looms.iter().map(|l| l.id.as_str()).collect();
    change.looms_added = new_loom_ids
        .iter()
        .filter(|id| !prev_loom_ids.contains(id))
        .map(|id| id.to_string())
        .collect();
    change.looms_removed = prev_loom_ids
        .iter()
        .filter(|id| !new_loom_ids.contains(id))
        .map(|id| id.to_string())
        .collect();

    // ── Knots (by id within looms present in both) ──
    for new_loom in &new.looms {
        let Some(prev_loom) = prev
            .looms
            .iter()
            .find(|l| l.id == new_loom.id)
        else {
            continue;
        };
        let prev_knot_ids: Vec<&str> =
            prev_loom.knots.iter().map(|k| k.id.as_str()).collect();
        let new_knot_ids: Vec<&str> =
            new_loom.knots.iter().map(|k| k.id.as_str()).collect();
        for id in &new_knot_ids {
            if !prev_knot_ids.contains(id) {
                change.knots_added.push((new_loom.id.as_str().to_string(), id.to_string()));
            }
        }
        for id in &prev_knot_ids {
            if !new_knot_ids.contains(id) {
                change.knots_removed.push((new_loom.id.as_str().to_string(), id.to_string()));
            }
        }
        // Field-level updates for knots present in both.
        for new_knot in &new_loom.knots {
            let Some(prev_knot) = prev_loom
                .knots
                .iter()
                .find(|k| k.id == new_knot.id)
            else {
                continue;
            };
            let mut field_changes: Vec<(KnotField, Option<String>, Option<String>)> = Vec::new();
            if prev_knot.status != new_knot.status {
                field_changes.push((
                    KnotField::Status,
                    Some(prev_knot.status.clone()),
                    Some(new_knot.status.clone()),
                ));
            }
            if prev_knot.last_strand_path != new_knot.last_strand_path {
                field_changes.push((
                    KnotField::LastStrandPath,
                    prev_knot.last_strand_path.clone(),
                    new_knot.last_strand_path.clone(),
                ));
            }
            if prev_knot.last_tie_off_path != new_knot.last_tie_off_path {
                field_changes.push((
                    KnotField::LastTieOffPath,
                    prev_knot.last_tie_off_path.clone(),
                    new_knot.last_tie_off_path.clone(),
                ));
            }
            if prev_knot.last_error != new_knot.last_error {
                field_changes.push((
                    KnotField::LastError,
                    prev_knot.last_error.clone(),
                    new_knot.last_error.clone(),
                ));
            }
            if prev_knot.last_event_at != new_knot.last_event_at {
                field_changes.push((
                    KnotField::LastEventAt,
                    prev_knot.last_event_at.clone(),
                    new_knot.last_event_at.clone(),
                ));
            }
            if !field_changes.is_empty() {
                change.knot_updates.push(KnotUpdate {
                    loom: new_loom.id.as_str().to_string(),
                    knot: new_knot.id.as_str().to_string(),
                    field_changes,
                });
            }
        }
    }

    // ── Profiles (by name) ──
    let prev_profile_names: Vec<&str> =
        prev.profiles.iter().map(|p| p.name.as_str()).collect();
    let new_profile_names: Vec<&str> =
        new.profiles.iter().map(|p| p.name.as_str()).collect();
    change.profiles_added = new_profile_names
        .iter()
        .filter(|name| !prev_profile_names.contains(name))
        .map(|name| name.to_string())
        .collect();
    change.profiles_removed = prev_profile_names
        .iter()
        .filter(|name| !new_profile_names.contains(name))
        .map(|name| name.to_string())
        .collect();

    for new_profile in &new.profiles {
        let Some(prev_profile) = prev
            .profiles
            .iter()
            .find(|p| p.name == new_profile.name)
        else {
            continue;
        };
        let mut field_changes: Vec<(ProfileField, Option<String>, Option<String>)> =
            Vec::new();
        let model_ref_changed = prev_profile.model_ref != new_profile.model_ref;
        if model_ref_changed {
            field_changes.push((
                ProfileField::ModelRef,
                prev_profile.model_ref.clone(),
                new_profile.model_ref.clone(),
            ));
        }
        let provider_changed = prev_profile.provider != new_profile.provider;
        if provider_changed {
            field_changes.push((
                ProfileField::Provider,
                prev_profile.provider.clone(),
                new_profile.provider.clone(),
            ));
        }
        let model_changed = prev_profile.model != new_profile.model;
        if model_changed {
            field_changes.push((
                ProfileField::Model,
                prev_profile.model.clone(),
                new_profile.model.clone(),
            ));
        }
        let thinking_changed = prev_profile.thinking_level != new_profile.thinking_level;
        if thinking_changed {
            field_changes.push((
                ProfileField::ThinkingLevel,
                prev_profile.thinking_level.map(|t| t.to_string()),
                new_profile.thinking_level.map(|t| t.to_string()),
            ));
        }
        let timeout_changed = prev_profile.timeout != new_profile.timeout;
        if timeout_changed {
            field_changes.push((
                ProfileField::Timeout,
                prev_profile.timeout.map(|t| t.to_string()),
                new_profile.timeout.map(|t| t.to_string()),
            ));
        }
        if !field_changes.is_empty() {
            change.profile_updates.push(ProfileUpdate {
                name: new_profile.name.as_str().to_string(),
                field_changes,
            });
        }
    }

    // ── Queue (by dedup key) ──
    change.queue_added = new
        .strand_queue
        .iter()
        .filter(|e| !prev.strand_queue.iter().any(|p| same_queue_key(p, e)))
        .cloned()
        .collect();
    change.queue_removed = prev
        .strand_queue
        .iter()
        .filter(|e| !new.strand_queue.iter().any(|n| same_queue_key(e, n)))
        .cloned()
        .collect();

    change
}

/// The queue's dedup key: `(strand_path, loom_id, knot_id, event_kind)`
/// — `queued_at` is excluded so a re-queued event is not reported as
/// both removed and added.
fn same_queue_key(a: &RigStateStrandQueueEntry, b: &RigStateStrandQueueEntry) -> bool {
    a.strand_path == b.strand_path
        && a.loom_id == b.loom_id
        && a.knot_id == b.knot_id
        && a.event_kind == b.event_kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{
        RigState, RigStateKnot, RigStateLoom, RigStateProfile, RigStateStrandQueueEntry,
    };

    fn base_state() -> RigState {
        RigState {
            rig_path: "/project/rig".into(),
            updated_at: "2026-09-07T00:00:00Z".into(),
            looms: vec![RigStateLoom {
                id: "review-loom".into(),
                knots: vec![RigStateKnot {
                    id: "review".into(),
                    status: "idle".into(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                }],
            }],
            profiles: vec![RigStateProfile {
                name: "fast".into(),
                model_ref: None,
                provider: Some("openai".into()),
                model: Some("gpt-4o".into()),
                thinking_level: None,
                timeout: None,
            }],
            strand_queue: vec![],
        }
    }

    #[test]
    fn diff_none_prev_is_empty_change() {
        let new = base_state();
        let change = diff_state(None, &new);
        assert_eq!(change, StateChange::default());
    }

    #[test]
    fn diff_identical_states_is_empty_change() {
        let prev = base_state();
        // `updated_at` differs but is excluded from the diff.
        let mut new = prev.clone();
        new.updated_at = "2026-09-07T00:05:00Z".into();
        let change = diff_state(Some(&prev), &new);
        assert_eq!(change, StateChange::default());
    }

    #[test]
    fn diff_detects_knot_status_and_field_changes() {
        let prev = base_state();
        let mut new = prev.clone();
        new.looms[0].knots[0].status = "processing".into();
        new.looms[0].knots[0].last_strand_path = Some("strands/prd.md".into());
        new.looms[0].knots[0].last_event_at = Some("2026-09-07T14:00:00Z".into());
        let change = diff_state(Some(&prev), &new);
        assert_eq!(change.knot_updates.len(), 1);
        let update = &change.knot_updates[0];
        assert_eq!(update.loom, "review-loom");
        assert_eq!(update.knot, "review");
        let fields: Vec<&KnotField> =
            update.field_changes.iter().map(|(f, _, _)| f).collect();
        assert!(fields.contains(&&KnotField::Status));
        assert!(fields.contains(&&KnotField::LastStrandPath));
        assert!(fields.contains(&&KnotField::LastEventAt));
        assert!(!fields.contains(&&KnotField::LastTieOffPath));
        assert!(!fields.contains(&&KnotField::LastError));
        let status_change = update
            .field_changes
            .iter()
            .find(|(f, _, _)| *f == KnotField::Status)
            .unwrap();
        assert_eq!(
            status_change.1,
            Some("idle".to_string())
        );
        assert_eq!(
            status_change.2,
            Some("processing".to_string())
        );
    }

    #[test]
    fn diff_detects_error_cleared() {
        let mut prev = base_state();
        prev.looms[0].knots[0].last_error = Some("boom".into());
        prev.looms[0].knots[0].status = "failed".into();
        let new = base_state();
        let change = diff_state(Some(&prev), &new);
        let update = &change.knot_updates[0];
        let error_change = update
            .field_changes
            .iter()
            .find(|(f, _, _)| *f == KnotField::LastError)
            .unwrap();
        assert_eq!(error_change.1, Some("boom".to_string()));
        assert_eq!(error_change.2, None);
    }

    #[test]
    fn diff_detects_loom_and_knot_addition_removal() {
        let prev = base_state();
        let mut new = prev.clone();
        new.looms.push(RigStateLoom {
            id: "docs-loom".into(),
            knots: vec![RigStateKnot {
                id: "write".into(),
                status: "idle".into(),
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
                last_event_at: None,
            }],
        });
        new.looms[0].knots[0].status = "removed-status-irrelevant".into();
        let change = diff_state(Some(&prev), &new);
        assert_eq!(change.looms_added, vec!["docs-loom".to_string()]);
        assert!(change.looms_removed.is_empty());
        // Knots in the new loom are not diffed (loom absent in prev).
        assert!(change.knots_added.is_empty());
    }

    #[test]
    fn diff_detects_profile_field_changes() {
        let prev = base_state();
        let mut new = prev.clone();
        new.profiles[0].model = Some("o3".into());
        new.profiles[0].thinking_level =
            Some(ThinkingLevel::XHigh);
        let change = diff_state(Some(&prev), &new);
        assert_eq!(change.profile_updates.len(), 1);
        let update = &change.profile_updates[0];
        assert_eq!(update.name, "fast");
        let model_change = update
            .field_changes
            .iter()
            .find(|(f, _, _)| *f == ProfileField::Model)
            .unwrap();
        assert_eq!(model_change.1, Some("gpt-4o".to_string()));
        assert_eq!(model_change.2, Some("o3".to_string()));
        let thinking_change = update
            .field_changes
            .iter()
            .find(|(f, _, _)| *f == ProfileField::ThinkingLevel)
            .unwrap();
        assert_eq!(thinking_change.1, None);
        assert_eq!(thinking_change.2, Some("xhigh".to_string()));
    }

    #[test]
    fn diff_detects_queue_add_and_remove() {
        let prev = base_state();
        let mut new = prev.clone();
        new.strand_queue = vec![RigStateStrandQueueEntry {
            strand_path: "strands/prd.md".into(),
            loom_id: "review-loom".into(),
            knot_id: "review".into(),
            event_kind: "created".into(),
            queued_at: "2026-09-07T14:00:00Z".into(),
        }];
        let change = diff_state(Some(&prev), &new);
        assert_eq!(change.queue_added.len(), 1);
        assert_eq!(
            change.queue_added[0].strand_path,
            "strands/prd.md"
        );
        assert!(change.queue_removed.is_empty());

        // Reversed: entry removed.
        let change = diff_state(Some(&new), &prev);
        assert_eq!(change.queue_removed.len(), 1);
        assert!(change.queue_added.is_empty());
    }

    #[test]
    fn queue_entry_with_changed_queued_at_is_not_a_change() {
        let mut prev = base_state();
        prev.strand_queue = vec![RigStateStrandQueueEntry {
            strand_path: "strands/prd.md".into(),
            loom_id: "review-loom".into(),
            knot_id: "review".into(),
            event_kind: "created".into(),
            queued_at: "2026-09-07T14:00:00Z".into(),
        }];
        let mut new = prev.clone();
        new.strand_queue[0].queued_at = "2026-09-07T14:05:00Z".into();
        let change = diff_state(Some(&prev), &new);
        assert!(change.queue_added.is_empty());
        assert!(change.queue_removed.is_empty());
    }
}
