use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export value objects for convenient access through the entities module
pub use crate::domain::value_objects::{AgentConfig, PromptTemplate, RigAgentConfig};

// ── Value Objects (identifiers and paths) ──────────────────────────────────

/// Unique identifier for a Knot.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema,
)]
pub struct KnotId(pub String);

/// Unique identifier for a Loom.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema,
)]
pub struct LoomId(pub String);

/// Path to a strand (input file being processed).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema,
)]
#[schema(value_type = String)]
pub struct StrandPath(pub PathBuf);

/// Path to a tie-off (output file produced).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema,
)]
#[schema(value_type = String)]
pub struct TieOffPath(pub PathBuf);

/// Status of a TieOff output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TieOffStatus {
    /// Output has been produced and written.
    Produced,
    /// Output failed to produce.
    Failed,
}

// ── Entities ───────────────────────────────────────────────────────────────

/// A Knot is the core unit of work: an agent goal paired with a prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Knot {
    pub id: KnotId,
    pub agent_config: AgentConfig,
    pub prompt_template: PromptTemplate,
    /// Directory to watch for strand files (required).
    #[schema(value_type = String)]
    pub strand_dir: PathBuf,
}

/// A Loom orchestrates a collection of Knots.
///
/// The loom directory is static and derived from the loom ID and rig base
/// path — not stored as a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Loom {
    pub id: LoomId,
    pub knots: Vec<Knot>,
}

/// A Strand is an input file being processed by a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Strand {
    pub path: StrandPath,
}

/// A TieOff is the output produced from processing a Strand with a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TieOff {
    pub content: String,
    pub path: TieOffPath,
    pub status: TieOffStatus,
    /// Optional event type metadata for append-mode sections.
    pub event_type: Option<String>,
    /// Optional strand path metadata for append-mode sections.
    pub strand_path: Option<String>,
    /// Optional timestamp for append-mode sections.
    pub timestamp: Option<String>,
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knot_construction() {
        let id = KnotId("prd-goals-review".to_string());
        let agent_config = AgentConfig {
            goal: "Review PRD goals for clarity".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            tools: Vec::new(),
        };
        let prompt_template = PromptTemplate {
            input_bundling: "full-file".to_string(),
            instructions: "Review the goals section.".to_string(),
        };

        let knot = Knot {
            id: id.clone(),
            agent_config: agent_config.clone(),
            prompt_template: prompt_template.clone(),
            strand_dir: PathBuf::from("strands"),
        };

        assert_eq!(knot.id, id);
        assert_eq!(knot.agent_config, agent_config);
        assert_eq!(knot.prompt_template, prompt_template);
        assert_eq!(knot.strand_dir, PathBuf::from("strands"));
    }

    #[test]
    fn knot_construction_with_strand_dir() {
        let knot = Knot {
            id: KnotId("custom-dirs".to_string()),
            agent_config: AgentConfig {
                goal: "Review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "Check it.".to_string(),
            },
            strand_dir: PathBuf::from("../custom-source"),
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
            agent_config: AgentConfig {
                goal: "Review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "Check it.".to_string(),
            },
            strand_dir: PathBuf::from("project/prds"),
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
    fn knot_serialization() {
        let knot = Knot {
            id: KnotId("test".to_string()),
            agent_config: AgentConfig {
                goal: "test goal".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "do it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
        };

        let json = serde_json::to_string(&knot).unwrap();
        let deserialized: Knot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot);
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
            event_type: None,
            strand_path: None,
            timestamp: None,
        };

        assert_eq!(tieoff.status, TieOffStatus::Failed);

        let json = serde_json::to_string(&tieoff).unwrap();
        let deserialized: TieOff = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, TieOffStatus::Failed);
    }
}
