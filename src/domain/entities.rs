use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export value objects for convenient access through the entities module
pub use crate::domain::value_objects::{AgentConfig, PromptTemplate, WorkspaceAgentConfig};

// ── Value Objects (identifiers and paths) ──────────────────────────────────

/// Unique identifier for a Knot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KnotId(pub String);

/// Unique identifier for a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LoomId(pub String);

/// Path to a strand (input file being processed).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StrandPath(pub PathBuf);

/// Path to a tie-off (output file produced).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TieOffPath(pub PathBuf);

/// Status of a TieOff output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TieOffStatus {
    /// Output has been produced and written.
    Produced,
    /// Output failed to produce.
    Failed,
}

// ── Entities ───────────────────────────────────────────────────────────────

/// A Knot is the core unit of work: an agent goal paired with a prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knot {
    pub id: KnotId,
    pub agent_config: AgentConfig,
    pub prompt_template: PromptTemplate,
}

/// A Loom orchestrates a collection of Knots over a source directory,
/// writing output to a tie-off directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loom {
    pub id: LoomId,
    pub source_dir: PathBuf,
    pub tie_off_dir: PathBuf,
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
        };

        assert_eq!(knot.id, id);
        assert_eq!(knot.agent_config, agent_config);
        assert_eq!(knot.prompt_template, prompt_template);
    }

    #[test]
    fn loom_construction() {
        let id = LoomId("prds".to_string());
        let source_dir = PathBuf::from("project/prds");
        let tie_off_dir = PathBuf::from("output/prds");
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
        }];

        let loom = Loom {
            id: id.clone(),
            source_dir: source_dir.clone(),
            tie_off_dir: tie_off_dir.clone(),
            knots: knots.clone(),
        };

        assert_eq!(loom.id, id);
        assert_eq!(loom.source_dir, source_dir);
        assert_eq!(loom.tie_off_dir, tie_off_dir);
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
        };

        assert_eq!(tieoff.content, content);
        assert_eq!(tieoff.path, path);
        assert_eq!(tieoff.status, status);
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
        };

        let json = serde_json::to_string(&knot).unwrap();
        let deserialized: Knot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot);
    }

    #[test]
    fn loom_serialization() {
        let loom = Loom {
            id: LoomId("test".to_string()),
            source_dir: PathBuf::from("src"),
            tie_off_dir: PathBuf::from("out"),
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
        };

        assert_eq!(tieoff.status, TieOffStatus::Failed);

        let json = serde_json::to_string(&tieoff).unwrap();
        let deserialized: TieOff = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, TieOffStatus::Failed);
    }
}
