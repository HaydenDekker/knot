use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export value objects for convenient access through the entities module
pub use crate::domain::value_objects::{PromptTemplate, RigAgentConfig};
pub use crate::domain::value_objects::AgentProfile;

// ── Value Objects (identifiers and paths) ──────────────────────────────────

/// Unique identifier for a Knot.
/// Unique identifier for a Knot.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
pub struct KnotId(pub String);

/// Unique identifier for a Loom.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
pub struct LoomId(pub String);

/// Path to a strand (input file being processed).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
pub struct StrandPath(pub PathBuf);

/// Path to a tie-off (output file produced).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
pub struct TieOffPath(pub PathBuf);

/// Path to the rig-log (append-only JSONL operational log at `rig/.rig-log`).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
pub struct RigLogPath(pub PathBuf);

/// Status of a TieOff output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TieOffStatus {
    /// Output has been produced and written.
    Produced,
    /// Output failed to produce.
    Failed,
}

// ── Entities ───────────────────────────────────────────────────────────────

/// Default value for `git_versioned`: enabled.
fn default_git_versioned() -> bool {
    true
}

/// A Knot is the core unit of work: an agent goal paired with a prompt template.
///
/// All agent configuration comes from a shared profile referenced by
/// `agent_profile_ref`. The knot's `prompt_template.instructions` provides
/// task-specific direction appended to the profile's system prompt.
/// A Knot is the core unit of work: an agent goal paired with a prompt template.
///
/// All agent configuration comes from a shared profile referenced by
/// `agent_profile_ref`. The knot's `prompt_template.instructions` provides
/// task-specific direction appended to the profile's system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knot {
    pub id: KnotId,
    /// Reference to a named agent profile stored in `profiles/{name}.md`.
    pub agent_profile_ref: String,
    pub prompt_template: PromptTemplate,
    /// Directory to watch for strand files (required).
    pub strand_dir: PathBuf,
    /// When `true` (default), a git commit is created after each successful
    /// knot run. Set to `false` in frontmatter via `git-versioned: false` to
    /// opt out of automatic versioning for this knot.
    #[serde(default = "default_git_versioned")]
    pub git_versioned: bool,
}

/// A Loom orchestrates a collection of Knots.
///
/// The loom directory is static and derived from the loom ID and rig base
/// path — not stored as a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loom {
    pub id: LoomId,
    pub knots: Vec<Knot>,
}

/// A Strand is an input file being processed by a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strand {
    pub path: StrandPath,
}

/// A TieOff is the output produced from processing a Strand with a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TieOff {
    pub content: String,
    pub path: TieOffPath,
    pub status: TieOffStatus,
    /// Knot name for the section header (e.g. `"review-docs"`).
    pub knot_name: Option<String>,
    /// Optional event type metadata for append-mode sections.
    pub event_type: Option<String>,
    /// Optional strand path metadata for append-mode sections.
    pub strand_path: Option<String>,
    /// Optional timestamp for append-mode sections.
    pub timestamp: Option<String>,
}

// ── RigState — File-first state snapshot ─────────────────────────────

/// Snapshot of the entire rig's runtime state, written to `rig/state.json`.
///
/// This is the replacement for the HTTP interface — external consumers
/// (skills, scripts, other tools) read this file instead of calling
/// REST endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigState {
    /// Absolute path to the rig directory.
    pub rig_path: String,
    /// All registered looms and their knots.
    pub looms: Vec<RigStateLoom>,
    /// All registered agent profiles.
    pub profiles: Vec<RigStateProfile>,
    /// ISO 8601 UTC timestamp of the last state write.
    pub updated_at: String,
}

/// A loom as represented in the state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigStateLoom {
    /// The loom's unique ID.
    pub id: String,
    /// Knots within this loom.
    pub knots: Vec<RigStateKnot>,
}

/// A knot as represented in the state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigStateKnot {
    /// The knot's unique ID.
    pub id: String,
    /// Current processing status.
    pub status: String,
    /// Path to the last strand processed (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_strand_path: Option<String>,
    /// Path to the last tie-off produced (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tie_off_path: Option<String>,
    /// Error message from the last failed processing (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// ISO 8601 UTC timestamp of the last event for this knot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<String>,
}

/// An agent profile as represented in the state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigStateProfile {
    /// Profile name.
    pub name: String,
    /// The LLM provider identifier.
    pub provider: String,
    /// The model name to use.
    pub model: String,
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knot_construction() {
        let id = KnotId("prd-goals-review".to_string());
        let prompt_template = PromptTemplate {
            instructions: "Review the goals section.".to_string(),
        };

        let knot = Knot {
            id: id.clone(),
            agent_profile_ref: "fast".to_string(),
            prompt_template: prompt_template.clone(),
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        };

        assert_eq!(knot.id, id);
        assert_eq!(knot.agent_profile_ref, "fast");
        assert_eq!(knot.prompt_template, prompt_template);
        assert_eq!(knot.strand_dir, PathBuf::from("strands"));
        assert!(knot.git_versioned);
    }

    #[test]
    fn knot_construction_with_strand_dir() {
        let knot = Knot {
            id: KnotId("custom-dirs".to_string()),
            agent_profile_ref: "detailed".to_string(),
            prompt_template: PromptTemplate {
                instructions: "Check it.".to_string(),
            },
            strand_dir: PathBuf::from("../custom-source"),
            git_versioned: true,
        };

        assert_eq!(
            knot.strand_dir,
            PathBuf::from("../custom-source")
        );
    }

    #[test]
    fn loom_construction() {
        let id = LoomId("prds-loom".to_string());
        let knots = vec![Knot {
            id: KnotId("review".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "Check it.".to_string(),
            },
            strand_dir: PathBuf::from("project/prds"),
            git_versioned: true,
        }];

        let loom = Loom {
            id: id.clone(),
            knots: knots.clone(),
        };

        assert_eq!(loom.id, id);
        assert_eq!(loom.knots, knots);
    }

    #[test]
    fn strand_construction() {
        let path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let strand = Strand { path: path.clone() };

        assert_eq!(strand.path, path);
    }

    #[test]
    fn tieoff_construction() {
        let content = "Reviewed output here.".to_string();
        let path = TieOffPath(PathBuf::from("output/review.md"));
        let status = TieOffStatus::Produced;

        let tieoff = TieOff {
            content: content.clone(),
            path: path.clone(),
            status: status.clone(),
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
        };

        assert_eq!(tieoff.content, content);
        assert_eq!(tieoff.path, path);
        assert_eq!(tieoff.status, status);
        assert!(tieoff.event_type.is_none());
        assert!(tieoff.strand_path.is_none());
        assert!(tieoff.timestamp.is_none());
    }

    #[test]
    fn knot_id_newtype() {
        let id = KnotId("k1".to_string());
        assert_eq!(id.0, "k1");
    }

    #[test]
    fn loom_id_newtype() {
        let id = LoomId("l1".to_string());
        assert_eq!(id.0, "l1");
    }

    #[test]
    fn strand_path_newtype() {
        let p = StrandPath(PathBuf::from("foo.md"));
        assert_eq!(p.0, PathBuf::from("foo.md"));
    }

    #[test]
    fn tieoff_path_newtype() {
        let p = TieOffPath(PathBuf::from("out.md"));
        assert_eq!(p.0, PathBuf::from("out.md"));
    }

    #[test]
    fn riglog_path_newtype() {
        let p = RigLogPath(PathBuf::from("rig/.rig-log"));
        assert_eq!(p.0, PathBuf::from("rig/.rig-log"));
    }

    #[test]
    fn knot_serialization() {
        let knot = Knot {
            id: KnotId("test".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "do it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        };

        let json = serde_json::to_string(&knot).unwrap();
        let deserialized: Knot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot);
    }

    #[test]
    fn knot_serialization_roundtrip_with_git_versioned() {
        // git_versioned: true
        let knot_true = Knot {
            id: KnotId("git-on".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "do it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        };
        let json = serde_json::to_string(&knot_true).unwrap();
        let deserialized: Knot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot_true);
        assert!(deserialized.git_versioned);

        // git_versioned: false
        let knot_false = Knot {
            id: KnotId("git-off".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "do it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: false,
        };
        let json = serde_json::to_string(&knot_false).unwrap();
        let deserialized: Knot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot_false);
        assert!(!deserialized.git_versioned);

        // Missing field in JSON defaults to true
        let knot_no_field = Knot {
            id: KnotId("git-default".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "do it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        };
        // Build JSON without the git_versioned field
        let json_minimal = r#"{"id":"git-default","agent_profile_ref":"fast","prompt_template":{"instructions":"do it"},"strand_dir":"strands"}"#;
        let deserialized: Knot = serde_json::from_str(json_minimal).unwrap();
        assert_eq!(deserialized.id, knot_no_field.id);
        assert!(deserialized.git_versioned, "missing field should default to true");
    }

    #[test]
    fn loom_serialization() {
        let loom = Loom {
            id: LoomId("test-loom".to_string()),
            knots: vec![],
        };

        let json = serde_json::to_string(&loom).unwrap();
        let deserialized: Loom = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom);
    }

    #[test]
    fn strand_serialization() {
        let strand = Strand {
            path: StrandPath(PathBuf::from("in.md")),
        };

        let json = serde_json::to_string(&strand).unwrap();
        let deserialized: Strand = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, strand);
    }

    #[test]
    fn tieoff_serialization() {
        let tieoff = TieOff {
            content: "output".to_string(),
            path: TieOffPath(PathBuf::from("out.md")),
            status: TieOffStatus::Produced,
            knot_name: Some("review".to_string()),
            event_type: Some("created".to_string()),
            strand_path: Some("in.md".to_string()),
            timestamp: Some("2026-01-01T00:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&tieoff).unwrap();
        let deserialized: TieOff = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, tieoff);
    }

    #[test]
    fn tieoff_status_failed() {
        let tieoff = TieOff {
            content: String::new(),
            path: TieOffPath(PathBuf::from("err.md")),
            status: TieOffStatus::Failed,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
        };

        assert_eq!(tieoff.status, TieOffStatus::Failed);

        let json = serde_json::to_string(&tieoff).unwrap();
        let deserialized: TieOff = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, TieOffStatus::Failed);
    }

    // ── RigState Tests ──────────────────────────────────────────────────

    #[test]
    fn rig_state_construction() {
        let state = RigState {
            rig_path: "/path/to/rig".to_string(),
            looms: vec![RigStateLoom {
                id: "my-loom".to_string(),
                knots: vec![RigStateKnot {
                    id: "review-knot".to_string(),
                    status: "idle".to_string(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                }],
            }],
            profiles: vec![RigStateProfile {
                name: "fast".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
            }],
            updated_at: "2026-06-18T12:00:00Z".to_string(),
        };

        assert_eq!(state.rig_path, "/path/to/rig");
        assert_eq!(state.looms.len(), 1);
        assert_eq!(state.looms[0].id, "my-loom");
        assert_eq!(state.looms[0].knots.len(), 1);
        assert_eq!(state.looms[0].knots[0].id, "review-knot");
        assert_eq!(state.looms[0].knots[0].status, "idle");
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].name, "fast");
    }

    #[test]
    fn rig_state_serialization_roundtrip() {
        let state = RigState {
            rig_path: "/path/to/rig".to_string(),
            looms: vec![RigStateLoom {
                id: "prds-loom".to_string(),
                knots: vec![RigStateKnot {
                    id: "review".to_string(),
                    status: "completed".to_string(),
                    last_strand_path: Some("input/prd.md".to_string()),
                    last_tie_off_path: Some("output/review.md".to_string()),
                    last_error: None,
                    last_event_at: Some("2026-06-18T10:00:00Z".to_string()),
                }],
            }],
            profiles: vec![RigStateProfile {
                name: "fast".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
            }],
            updated_at: "2026-06-18T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: RigState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, state);
    }

    #[test]
    fn rig_state_serialization_omits_null_fields() {
        let state = RigState {
            rig_path: "/rig".to_string(),
            looms: vec![RigStateLoom {
                id: "loom".to_string(),
                knots: vec![RigStateKnot {
                    id: "k1".to_string(),
                    status: "idle".to_string(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                }],
            }],
            profiles: vec![],
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&state).unwrap();
        // None fields should be omitted, not serialized as null
        assert!(!json.contains("null"));
    }

    #[test]
    fn rig_state_empty() {
        let state = RigState {
            rig_path: "/empty/rig".to_string(),
            looms: vec![],
            profiles: vec![],
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: RigState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, state);
        assert!(deserialized.looms.is_empty());
        assert!(deserialized.profiles.is_empty());
    }

    #[test]
    fn rig_state_knot_with_error() {
        let knot = RigStateKnot {
            id: "k1".to_string(),
            status: "failed".to_string(),
            last_strand_path: Some("input.md".to_string()),
            last_tie_off_path: None,
            last_error: Some("timeout".to_string()),
            last_event_at: Some("2026-06-18T10:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&knot).unwrap();
        let deserialized: RigStateKnot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot);
        assert_eq!(deserialized.last_error, Some("timeout".to_string()));
    }

    #[test]
    fn rig_state_multiple_looms_and_knots() {
        let state = RigState {
            rig_path: "/multi".to_string(),
            looms: vec![
                RigStateLoom {
                    id: "loom-a".to_string(),
                    knots: vec![
                        RigStateKnot {
                            id: "k1".to_string(),
                            status: "idle".to_string(),
                            last_strand_path: None,
                            last_tie_off_path: None,
                            last_error: None,
                            last_event_at: None,
                        },
                        RigStateKnot {
                            id: "k2".to_string(),
                            status: "processing".to_string(),
                            last_strand_path: Some("in.md".to_string()),
                            last_tie_off_path: None,
                            last_error: None,
                            last_event_at: Some("2026-06-18T10:00:00Z".to_string()),
                        },
                    ],
                },
                RigStateLoom {
                    id: "loom-b".to_string(),
                    knots: vec![],
                },
            ],
            profiles: vec![
                RigStateProfile {
                    name: "fast".to_string(),
                    provider: "openai".to_string(),
                    model: "gpt-4o".to_string(),
                },
                RigStateProfile {
                    name: "detailed".to_string(),
                    provider: "anthropic".to_string(),
                    model: "claude-sonnet".to_string(),
                },
            ],
            updated_at: "2026-06-18T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: RigState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, state);
        assert_eq!(deserialized.looms.len(), 2);
        assert_eq!(deserialized.looms[0].knots.len(), 2);
        assert_eq!(deserialized.looms[1].knots.len(), 0);
        assert_eq!(deserialized.profiles.len(), 2);
    }
}
