use serde::{Deserialize, Serialize};

use crate::domain::entities::{
    Knot, KnotId, LoomId, StrandPath, TieOffPath,
};

// ── Domain Events ──────────────────────────────────────────────────────────

/// An event that describes the lifecycle of a Strand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum StrandEvent {
    /// A new strand (input file) was detected.
    Created {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
    },
    /// An existing strand was modified.
    Modified {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
    },
    /// A strand was removed from the source.
    Deleted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
    },
}

/// A TieOff (output file) was successfully produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TieOffProduced {
    pub knot_id: KnotId,
    pub strand_path: StrandPath,
    pub tie_off_path: TieOffPath,
}

/// Processing of a strand failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProcessingFailed {
    pub knot_id: KnotId,
    pub strand_path: StrandPath,
    pub error_message: String,
}

/// An event that describes the lifecycle of a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum LoomEvent {
    /// A new Knot was registered with the Loom.
    KnotRegistered {
        loom_id: LoomId,
        knot_id: KnotId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// The Loom began processing its strands.
    LoomStarted {
        loom_id: LoomId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// The Loom stopped processing.
    LoomStopped {
        loom_id: LoomId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A strand was processed (either produced output or failed).
    StrandProcessed {
        loom_id: LoomId,
        strand_path: StrandPath,
        /// Error message if processing failed. `None` on success.
        error: Option<String>,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot started processing a strand.
    KnotProcessing {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot completed processing a strand successfully.
    KnotCompleted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        tie_off_path: TieOffPath,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot failed while processing a strand.
    KnotFailed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        error: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot was deregistered from the loom.
    KnotDeregistered {
        loom_id: LoomId,
        knot_id: KnotId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot file contained unknown YAML properties (accepted but not used).
    KnotParseWarning {
        loom_id: LoomId,
        knot_file_name: String,
        message: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
}

/// A Knot was registered with a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KnotRegistered {
    pub loom_id: LoomId,
    pub knot_id: KnotId,
}

// ── Rig-Log Events ─────────────────────────────────────────────────────────

/// An operational event written to the rig-log (`rig/.rig-log`).
///
/// The rig-log is an append-only JSONL file that records serious operational
/// events so the user or an external watcher can monitor and react.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum RigLogEvent {
    /// An agent session exceeded its timeout deadline.
    TimeoutExceeded {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        error: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// All pending events have been processed and the queue is idle.
    QueueIdle {
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
}

// ── Configuration Events ───────────────────────────────────────────────────

/// An event that describes configuration changes to looms and knots.
///
/// Unlike [`StrandEvent`] which tracks input file lifecycle, config events
/// track changes to the loom/knot definition files themselves (the `.md` knot
/// files and `*-loom` directories).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum ConfigEvent {
    /// A new loom directory was detected (ends in `-loom`).
    LoomAdded {
        loom_id: LoomId,
        /// Absolute path to the loom directory (e.g. `/project/rig/new-loom`).
        /// Used by `ConfigEventHandler` to scan only this directory instead of
        /// re-scanning the full rig.
        loom_dir: String,
    },
    /// A new knot `.md` file was created in a loom directory.
    KnotAdded {
        loom_id: LoomId,
        knot: Knot,
    },
    /// An existing knot `.md` file was modified in a loom directory.
    KnotModified {
        loom_id: LoomId,
        knot: Knot,
    },
    /// A knot `.md` file was deleted from a loom directory.
    KnotDeleted {
        loom_id: LoomId,
        knot_id: KnotId,
    },
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::PromptTemplate;
    use std::path::PathBuf;

    #[test]
    fn strand_event_types() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));

        let created = StrandEvent::Created {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
        };
        let modified = StrandEvent::Modified {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
        };
        let deleted = StrandEvent::Deleted {
            loom_id,
            knot_id,
            strand_path,
        };

        // Verify all three variants exist and carry correct data
        match created {
            StrandEvent::Created {
                loom_id: ref lid,
                knot_id: ref kid,
                strand_path: ref sp,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(*kid, KnotId("review".to_string()));
                assert_eq!(sp.0, PathBuf::from("project/prds/my-prd.md"));
            }
            _ => panic!("Expected Created variant"),
        }

        match modified {
            StrandEvent::Modified {
                loom_id: ref lid,
                knot_id: ref kid,
                strand_path: ref sp,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(*kid, KnotId("review".to_string()));
                assert_eq!(sp.0, PathBuf::from("project/prds/my-prd.md"));
            }
            _ => panic!("Expected Modified variant"),
        }

        match deleted {
            StrandEvent::Deleted {
                loom_id: ref lid,
                knot_id: ref kid,
                strand_path: ref sp,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(*kid, KnotId("review".to_string()));
                assert_eq!(sp.0, PathBuf::from("project/prds/my-prd.md"));
            }
            _ => panic!("Expected Deleted variant"),
        }
    }

    #[test]
    fn tieoff_produced_event() {
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let tie_off_path = TieOffPath(PathBuf::from("output/review.md"));

        let event = TieOffProduced {
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            tie_off_path: tie_off_path.clone(),
        };

        assert_eq!(event.knot_id, knot_id);
        assert_eq!(event.strand_path, strand_path);
        assert_eq!(event.tie_off_path, tie_off_path);

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: TieOffProduced = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn processing_failed_event() {
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let error_message = "Agent returned non-zero exit code".to_string();

        let event = ProcessingFailed {
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            error_message: error_message.clone(),
        };

        assert_eq!(event.knot_id, knot_id);
        assert_eq!(event.strand_path, strand_path);
        assert_eq!(event.error_message, error_message);

        // Verify error details are preserved through serialisation
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ProcessingFailed = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.error_message, error_message);
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_types() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));

        let ts = "2026-06-10T12:00:00Z".to_string();
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            timestamp: ts.clone(),
        };
        let loom_started = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: ts.clone(),
        };
        let loom_stopped = LoomEvent::LoomStopped {
            loom_id: loom_id.clone(),
            timestamp: ts.clone(),
        };
        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: loom_id.clone(),
            strand_path: strand_path.clone(),
            error: None,
            timestamp: ts.clone(),
        };

        // Verify KnotRegistered
        match knot_registered {
            LoomEvent::KnotRegistered {
                loom_id: ref lid,
                knot_id: ref kid,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected KnotRegistered variant"),
        }

        // Verify LoomStarted
        match loom_started {
            LoomEvent::LoomStarted {
                loom_id: ref lid,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected LoomStarted variant"),
        }

        // Verify LoomStopped
        match loom_stopped {
            LoomEvent::LoomStopped {
                loom_id: ref lid,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected LoomStopped variant"),
        }

        // Verify StrandProcessed
        match strand_processed {
            LoomEvent::StrandProcessed {
                loom_id: ref lid,
                strand_path: ref sp,
                error,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*sp, strand_path);
                assert!(error.is_none());
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected StrandProcessed variant"),
        }
    }

    #[test]
    fn knot_registered_event() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());

        let event = KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        };

        assert_eq!(event.loom_id, loom_id);
        assert_eq!(event.knot_id, knot_id);

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: KnotRegistered = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn strand_event_serialisation() {
        let created = StrandEvent::Created {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
        };

        let json = serde_json::to_string(&created).unwrap();
        let deserialized: StrandEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, created);

        // Also verify Modified and Deleted round-trip
        let modified = StrandEvent::Modified {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
        };
        let json = serde_json::to_string(&modified).unwrap();
        let deserialized: StrandEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, modified);

        let deleted = StrandEvent::Deleted {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
        };
        let json = serde_json::to_string(&deleted).unwrap();
        let deserialized: StrandEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, deleted);
    }

    #[test]
    fn loom_event_serialisation() {
        let ts = "2026-06-10T12:00:00Z".to_string();
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&knot_registered).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot_registered);

        let loom_started = LoomEvent::LoomStarted {
            loom_id: LoomId("prds".to_string()),
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&loom_started).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom_started);

        let loom_stopped = LoomEvent::LoomStopped {
            loom_id: LoomId("prds".to_string()),
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&loom_stopped).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom_stopped);

        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: LoomId("prds".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            error: None,
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&strand_processed).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, strand_processed);
    }

    #[test]
    fn loom_event_strand_processed_with_error() {
        let event = LoomEvent::StrandProcessed {
            loom_id: LoomId("prds".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            error: Some("agent crashed".to_string()),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };

        // Verify error field is present
        match &event {
            LoomEvent::StrandProcessed { error, .. } => {
                assert_eq!(error.as_deref(), Some("agent crashed"));
            }
            _ => panic!("Expected StrandProcessed"),
        }

        // Verify error survives serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_processing() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotProcessing {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotProcessing variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_completed() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let tie_off_path = TieOffPath(PathBuf::from("output/review.md"));
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotCompleted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            tie_off_path: tie_off_path.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotCompleted {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                tie_off_path: tp,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(*tp, tie_off_path);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotCompleted variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_failed() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let error = "Agent returned non-zero exit code".to_string();
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotFailed {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            error: error.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotFailed {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                error: msg,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(msg.as_str(), error);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotFailed variant"),
        }

        // Verify error survives serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    fn make_knot(id: &str) -> Knot {
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "Test instructions.".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// `ConfigEvent::LoomAdded` carries both `loom_id` and `loom_dir`.
    /// Verifies the variant shape and JSON round-trip serialisation.
    #[test]
    fn config_event_loom_added_has_path() {
        let loom_id = LoomId("my-loom".to_string());
        let loom_dir = "/project/rig/my-loom".to_string();

        let event = ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: loom_dir.clone(),
        };

        // Verify both fields are present
        match &event {
            ConfigEvent::LoomAdded {
                loom_id: lid,
                loom_dir: dir,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(dir, &loom_dir);
            }
            _ => panic!("Expected LoomAdded variant"),
        }

        // Verify JSON serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConfigEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn config_event_types() {
        let loom_id = LoomId("prds".to_string());
        let knot = make_knot("review");
        let knot_id = KnotId("review".to_string());

        // Build all four variants
        let loom_added = ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: "/project/rig/prds-loom".to_string(),
        };
        let knot_added = ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot: knot.clone(),
        };
        let knot_modified = ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: knot.clone(),
        };
        let knot_deleted = ConfigEvent::KnotDeleted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        };

        // Verify LoomAdded carries correct data
        match &loom_added {
            ConfigEvent::LoomAdded {
                loom_id: lid,
                loom_dir,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(loom_dir, &"/project/rig/prds-loom".to_string());
            }
            _ => panic!("Expected LoomAdded variant"),
        }

        // Verify KnotAdded carries correct data
        match &knot_added {
            ConfigEvent::KnotAdded {
                loom_id: lid,
                knot: k,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(k.id, KnotId("review".to_string()));
            }
            _ => panic!("Expected KnotAdded variant"),
        }

        // Verify KnotModified carries correct data
        match &knot_modified {
            ConfigEvent::KnotModified {
                loom_id: lid,
                knot: k,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(k.id, KnotId("review".to_string()));
            }
            _ => panic!("Expected KnotModified variant"),
        }

        // Verify KnotDeleted carries correct data
        match &knot_deleted {
            ConfigEvent::KnotDeleted {
                loom_id: lid,
                knot_id: kid,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
            }
            _ => panic!("Expected KnotDeleted variant"),
        }

        // Verify serialisation round-trip for all variants
        let events: Vec<ConfigEvent> =
            vec![loom_added, knot_added, knot_modified, knot_deleted];
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: ConfigEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(
                deserialized, *event,
                "round-trip failed for variant"
            );
        }
    }

    #[test]
    fn loom_event_serialisation_all_variants() {
        // Verify all 9 variants round-trip through JSON
        let loom_id = LoomId("all".to_string());
        let knot_id = KnotId("k1".to_string());
        let strand_path = StrandPath(PathBuf::from("in.md"));
        let tie_off_path = TieOffPath(PathBuf::from("out.md"));
        let ts = "2026-06-10T12:00:00Z".to_string();

        let events: Vec<LoomEvent> = vec![
            LoomEvent::KnotRegistered {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::LoomStarted {
                loom_id: loom_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::LoomStopped {
                loom_id: loom_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::StrandProcessed {
                loom_id: loom_id.clone(),
                strand_path: strand_path.clone(),
                error: None,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotProcessing {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotCompleted {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                tie_off_path: tie_off_path.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotFailed {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                error: "boom".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotDeregistered {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotParseWarning {
                loom_id: loom_id.clone(),
                knot_file_name: "legacy.md".to_string(),
                message: "unknown property 'tie-off-dir'".to_string(),
                timestamp: ts.clone(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, *event, "round-trip failed for variant");
        }
    }

    #[test]
    fn loom_event_knot_parse_warning() {
        let loom_id = LoomId("prds".to_string());
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotParseWarning {
            loom_id: loom_id.clone(),
            knot_file_name: "legacy-knot.md".to_string(),
            message: "unknown property 'tie-off-dir' in knot frontmatter (not used)".to_string(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotParseWarning {
                loom_id: lid,
                knot_file_name,
                message,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*knot_file_name, "legacy-knot.md");
                assert!(message.contains("tie-off-dir"));
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotParseWarning variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn riglog_event_timeout_exceeded() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let error = "Agent session exceeded 60s deadline".to_string();
        let ts = "2026-06-14T10:00:00Z".to_string();

        let event = RigLogEvent::TimeoutExceeded {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            error: error.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            RigLogEvent::TimeoutExceeded {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                error: msg,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(msg.as_str(), error);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected TimeoutExceeded variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RigLogEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn riglog_event_queue_idle() {
        let ts = "2026-06-14T10:05:00Z".to_string();

        let event = RigLogEvent::QueueIdle {
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            RigLogEvent::QueueIdle { timestamp: t } => {
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected QueueIdle variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RigLogEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn riglog_event_serialisation_all_variants() {
        let loom_id = LoomId("ops".to_string());
        let knot_id = KnotId("slow-review".to_string());
        let strand_path = StrandPath(PathBuf::from("input/data.md"));
        let ts = "2026-06-14T12:00:00Z".to_string();

        let events: Vec<RigLogEvent> = vec![
            RigLogEvent::TimeoutExceeded {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                error: "deadline exceeded after 600s".to_string(),
                timestamp: ts.clone(),
            },
            RigLogEvent::QueueIdle {
                timestamp: ts.clone(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: RigLogEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, *event, "round-trip failed for variant");
        }
    }
}
