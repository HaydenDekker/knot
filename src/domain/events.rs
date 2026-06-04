use serde::{Deserialize, Serialize};

use crate::domain::entities::{KnotId, LoomId, StrandPath, TieOffPath};

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
    },
    /// The Loom began processing its strands.
    LoomStarted {
        loom_id: LoomId,
    },
    /// The Loom stopped processing.
    LoomStopped {
        loom_id: LoomId,
    },
    /// A strand was processed (either produced output or failed).
    StrandProcessed {
        loom_id: LoomId,
        strand_path: StrandPath,
        /// Error message if processing failed. `None` on success.
        error: Option<String>,
    },
}

/// A Knot was registered with a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KnotRegistered {
    pub loom_id: LoomId,
    pub knot_id: KnotId,
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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

        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        };
        let loom_started = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
        };
        let loom_stopped = LoomEvent::LoomStopped {
            loom_id: loom_id.clone(),
        };
        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: loom_id.clone(),
            strand_path: strand_path.clone(),
            error: None,
        };

        // Verify KnotRegistered
        match knot_registered {
            LoomEvent::KnotRegistered {
                loom_id: ref lid,
                knot_id: ref kid,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
            }
            _ => panic!("Expected KnotRegistered variant"),
        }

        // Verify LoomStarted
        match loom_started {
            LoomEvent::LoomStarted { loom_id: ref lid } => {
                assert_eq!(*lid, loom_id);
            }
            _ => panic!("Expected LoomStarted variant"),
        }

        // Verify LoomStopped
        match loom_stopped {
            LoomEvent::LoomStopped { loom_id: ref lid } => {
                assert_eq!(*lid, loom_id);
            }
            _ => panic!("Expected LoomStopped variant"),
        }

        // Verify StrandProcessed
        match strand_processed {
            LoomEvent::StrandProcessed {
                loom_id: ref lid,
                strand_path: ref sp,
                error,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*sp, strand_path);
                assert!(error.is_none());
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
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
        };
        let json = serde_json::to_string(&knot_registered).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot_registered);

        let loom_started = LoomEvent::LoomStarted {
            loom_id: LoomId("prds".to_string()),
        };
        let json = serde_json::to_string(&loom_started).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom_started);

        let loom_stopped = LoomEvent::LoomStopped {
            loom_id: LoomId("prds".to_string()),
        };
        let json = serde_json::to_string(&loom_stopped).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom_stopped);

        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: LoomId("prds".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            error: None,
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
}
