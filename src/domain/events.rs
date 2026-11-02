use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::entities::{
    Knot, KnotId, LoomId, StrandPath, TieOffPath,
};
use crate::domain::pending_event::QueueEntryView;

// ── Agent Events (Intent-Based Routing) ────────────────────────────────────

use std::collections::HashMap;



/// A structured agent-to-agent event emitted in a tie-off.
///
/// When a producer knot is instructed to emit events (via intent-based routing
/// context injection), it writes one structured block per subscriber event
/// in its tie-off body. Each block carries an `occurred` flag indicating
/// whether the event actually happened during the session.
///
/// Events with `occurred = false` are not dispatched to consumers but still
/// count as acknowledgement of the subscriber requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Unique event identifier (e.g. `PlanCreated`).
    pub event_id: String,
    /// Whether the event actually occurred during the session.
    ///
    /// `false` events are not dispatched to consumers but still count as
    /// acknowledgement of the subscriber requirement.
    ///
    /// Defaults to `true` for backwards compatibility with events that
    /// omit this field.
    #[serde(default = "default_occurred")]
    pub occurred: bool,
    /// Arbitrary key-value pairs carrying event data.
    /// Includes fields like `plan`, `description`, `source`, etc.
    #[serde(default)]
    pub payload: HashMap<String, String>,
    /// Freeform narrative context attached to the event.
    ///
    /// When agents emit events inside ```markdown code blocks with
    /// YAML-style frontmatter, the text after the closing `---` is
    /// captured here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

fn default_occurred() -> bool {
    true
}



// ── Context Provider Abstraction ─────────────────────────────────────

/// Trait for accessing strand event queue state.
///
/// Implemented by the application-layer queue so the domain can query
/// which strand paths are currently pending (in-queue but not yet
/// processed) without depending on the concrete queue type.
///
/// Used by [`ContextProvider`] implementations to determine which
/// dispatched events are still pending vs. already consumed, and by
/// `ProcessStrand` to remove the event it just processed (late
/// removal — the file is deleted after the work is done, not when it
/// is read for processing).
pub trait StrandQueueAccessor: Send + Sync + std::fmt::Debug {
    /// Return the strand paths currently sitting in the queue
    /// (debounced, awaiting processing).
    fn pending_strand_paths(&self) -> Vec<std::path::PathBuf>;

    /// Return the queue's current entries (path + `queued_at`) in FIFO
    /// order.
    ///
    /// Used by the producer's queued-events index (plan 097), which needs
    /// the queue's own `queued_at` timestamps. Default: derive from
    /// [`Self::pending_strand_paths`] with an unknown (empty) timestamp —
    /// implementations backed by real queue entries override with the
    /// persisted timestamp.
    fn pending_entries(&self) -> Vec<QueueEntryView> {
        self.pending_strand_paths()
            .into_iter()
            .map(|strand_path| QueueEntryView {
                strand_path,
                queued_at: String::new(),
                event_id: None,
                producer: None,
            })
            .collect()
    }

    /// Remove the queued event with the given ID.
    ///
    /// Called by `ProcessStrand` as the explicit removal step of the
    /// late-removal (at-least-once) semantics: on success the file is
    /// deleted just before the git commit (the commit captures
    /// everything, including the removal); on failure/skip it is
    /// deleted at the point of failure. Returns `true` if the event
    /// was found and removed, `false` if it was already gone.
    fn delete(&self, id: &crate::domain::pending_event::PendingEventId) -> bool;
}

/// Trait for reading per-session prompt instruction content (the two
/// overridable instruction **documents** — the `knot-template` skill's
/// two content templates).
///
/// The two documents, each with a fixed injection scope:
///
/// - **Doc 1 — the tie-off instruction** (`read_tie_off_instruction`,
///   the `## Tie-Off Content` section — the tie-off's header/narrative
///   content). Injected into **every** knot, every new session —
///   unconditional (the `# Tie-Off` block).
/// - **Doc 2 — the event-content instruction**
///   (`read_event_content_instruction`, the `## Event Content Guide`
///   section — how to fill in the event blocks). Injected **only when
///   the knot has subscriber events** (the `# Subscriber Events`
///   block).
///
/// The hard-coded cores — the universal tie-off contract sentence
/// ([`DEFAULT_TIE_OFF_CONTRACT`]) and the event acknowledgement / format
/// / rules ([`DEFAULT_EVENT_CONTRACT`]) — are never overridable; this
/// source supplies the two content sections only.
///
/// Every call is a fresh read — each instruction is pulled each new
/// session (never cached from startup or a previous run). A `None`
/// result means "no per-rig link available": the caller falls back to
/// the hard-coded embedded default for **that section**
/// ([`DEFAULT_TIE_OFF_CONTENT_INSTRUCTION`] / [`DEFAULT_EVENT_CONTENT_INSTRUCTION`])
/// — the other section is unaffected.
pub trait PromptInstructionSource: Send + Sync + std::fmt::Debug {
    /// The tie-off instruction (doc 1 — the `## Tie-Off Content`
    /// section) for a new session: the file linked in the rig config
    /// (`tie-off-instruction-file`), or `None` for the hard-coded
    /// default tie-off content section.
    fn read_tie_off_instruction(&self) -> Option<String>;

    /// The event-content instruction (doc 2 — the `## Event Content
    /// Guide` section) for a new session: the file linked in the rig
    /// config (`event-content-instruction-file`), or `None` for the
    /// hard-coded default event-content guide.
    fn read_event_content_instruction(&self) -> Option<String>;
}

/// Data required to build dynamic prompt context segments.
///
/// Carries the knot being invoked, the loom it belongs to, all registered
/// knots (so providers can inspect consumer relationships), the rig
/// directory path, and an optional reference to the strand event queue
/// (so providers can determine which events are still pending).
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// The knot currently being invoked.
    pub knot: Knot,
    /// The loom containing the current knot.
    pub loom_id: LoomId,
    /// All registered knots across all looms.
    pub all_knots: Vec<Knot>,
    /// Absolute path to the rig directory.
    pub rig_dir: std::path::PathBuf,
    /// Optional reference to the strand event queue for pending event
    /// visibility. When present, pending events are determined by
    /// querying the in-memory queue (source of truth) instead of
    /// scanning the filesystem.
    pub strand_queue: Option<Arc<dyn StrandQueueAccessor>>,
    /// Plan 086: the self-continuation `TasksIncomplete` description to
    /// include in the `# Subscriber Events` block. `Some(desc)` when the
    /// knot's profile alias carries `ctx-wrap-up-limit` (the water-mark
    /// note can fire, so the format must be known before it can).
    /// `None` for non-water-marked aliases (the block is unchanged).
    pub self_continuation_desc: Option<String>,
    /// Per-session instruction source (the two overridable instruction
    /// **documents** — the `knot-template` skill's two content
    /// templates: the tie-off content section and the event-content
    /// guide). The hard-coded cores ([`DEFAULT_TIE_OFF_CONTRACT`] / [`DEFAULT_EVENT_CONTRACT`])
    /// are never sourced. `None` when not wired (unit tests) — the
    /// hard-coded embedded defaults for both sections are used. Fetched
    /// fresh on every build (two pulls): each is pulled each new
    /// session, never cached.
    pub instruction_source: Option<Arc<dyn PromptInstructionSource>>,
}

/// A provider that builds dynamic prompt context segments.
///
/// The domain defines the interface; concrete implementations live in the
/// application layer where they have access to the filesystem and rig state.
///
/// This is the first step in moving context injection from a single free
/// function (`build_listener_context`) to a composable, state-aware pipeline.
pub trait ContextProvider {
    /// Build a markdown context segment to be prepended to the agent prompt.
    ///
    /// Returns an empty string when no context is relevant.
    fn build_context(&self, input: &BuildContext) -> String;
}

// ── Context Injection ──────────────────────────────────────────────────────

/// The hard-coded universal tie-off contract (the
/// `knot-tie-off-contract` template) — the `## Tie-Off Contract`
/// section of the `# Tie-Off` block.
///
/// The universal sentence: the final response *is* the tie-off, and
/// `tie-offs/` files are never written or read directly. Injected
/// **unconditionally** into every knot's prompt, every new session
/// (leaf knots included) — the tie-off concern is not scoped to
/// producer status. This text is **never overridable per rig**: the
/// per-rig link (`tie-off-instruction-file`) reaches only the
/// tie-off content section ([`DEFAULT_TIE_OFF_CONTENT_INSTRUCTION`]).
///
/// The text lives in this repository at
/// `.agents/skills/knot-template/knot-tie-off-contract.md` — the
/// `knot-template` skill's contract reference (reference only, never
/// copied into the project), deployed alongside it to
/// `~/.agents/skills-library/knot-template/` — and is compiled in
/// here: this constant is the single source of truth.
///
/// User-facing agent text — keep it greppable. The template file is the
/// source of truth; edit it there.
pub const DEFAULT_TIE_OFF_CONTRACT: &str =
    include_str!("../../.agents/skills/knot-template/knot-tie-off-contract.md");

/// The hard-coded event contract (the `knot-event-contract` template)
/// — the core of the `# Subscriber Events` block.
///
/// The subscriber acknowledgement ("you have a number of subscribers…"
/// — each event must be acknowledged in the tie-off), the event format
/// (the fixed `event:` / `occurred:` / `timestamp:` block), and the
/// rules (one structured block per subscriber event — always, even
/// when `occurred: false`). Injected **only when the knot has
/// subscriber events** (or a self-continuation entry) — the format and
/// rules only make sense for knots that emit. This text is **never
/// overridable per rig**: the per-rig link
/// (`event-content-instruction-file`) reaches only the event-content
/// guide ([`DEFAULT_EVENT_CONTENT_INSTRUCTION`]).
///
/// The text lives in this repository at
/// `.agents/skills/knot-template/knot-event-contract.md` — the
/// `knot-template` skill's contract reference (reference only, never
/// copied into the project), deployed alongside it to
/// `~/.agents/skills-library/knot-template/` — and is compiled in
/// here: this constant is the single source of truth.
///
/// User-facing agent text — keep it greppable. The template file is the
/// source of truth; edit it there.
pub const DEFAULT_EVENT_CONTRACT: &str =
    include_str!("../../.agents/skills/knot-template/knot-event-contract.md");

/// The default tie-off instruction (doc 1 — the `knot-tie-off-content`
/// template: the `## Tie-Off Content` section only).
///
/// The overridable part of the `# Tie-Off` block: the tie-off's
/// header/narrative content — how to write the agent's final response.
/// Injected into **every** knot, every new session (unconditional).
///
/// Single source of truth: the text lives in this repository at
/// `.agents/skills/knot-template/knot-tie-off-content.md` (the
/// `knot-template` skill's tie-off content template, deployed
/// alongside it to `~/.agents/skills-library/knot-template/`) and is
/// compiled in here. `FileSystemPromptInstructionSource` resolves the
/// per-session text fresh (never cached; the service never writes into
/// the rig): a per-rig link named in
/// `.workspace-agent-config.yaml` (`tie-off-instruction-file`) wins if
/// readable, otherwise this embedded default section is injected. The
/// `knot-template` skill is the agent-readable procedure for creating
/// that per-rig override.
///
/// User-facing agent text — keep it greppable. The template file is the
/// source of truth; edit it there.
pub const DEFAULT_TIE_OFF_CONTENT_INSTRUCTION: &str =
    include_str!("../../.agents/skills/knot-template/knot-tie-off-content.md");

/// The default event-content instruction (doc 2 — the
/// `knot-event-content` template: the `## Event Content Guide` section
/// only).
///
/// The overridable part of the `# Subscriber Events` block: how to fill
/// in each event block's content — the `description` field, the
/// narrative body, and the extra fields a subscriber declares in its
/// `event_description`. Injected **only when the knot has subscriber
/// events** (or a self-continuation entry).
///
/// Single source of truth: the text lives in this repository at
/// `.agents/skills/knot-template/knot-event-content.md` (the
/// `knot-template` skill's event-content template, deployed alongside
/// it to `~/.agents/skills-library/knot-template/`) and is compiled in
/// here. `FileSystemPromptInstructionSource` resolves the per-session
/// text fresh (never cached; the service never writes into the rig):
/// a per-rig link named in `.workspace-agent-config.yaml`
/// (`event-content-instruction-file`) wins if readable, otherwise this
/// embedded default guide is injected. A failure of this link never
/// affects the tie-off instruction (per-section fallback).
///
/// User-facing agent text — keep it greppable. The template file is the
/// source of truth; edit it there.
pub const DEFAULT_EVENT_CONTENT_INSTRUCTION: &str =
    include_str!("../../.agents/skills/knot-template/knot-event-content.md");

/// Build the **unconditional** `# Tie-Off` block injected at the start
/// of **every** knot's prompt, every new session — leaf knots included.
///
/// Shape: the `# Tie-Off` header, the hard-coded universal contract
/// ([`DEFAULT_TIE_OFF_CONTRACT`], the `## Tie-Off Contract` section —
/// never replaced), the tie-off content section (doc 1 —
/// `tie_off_content_instruction` when `Some` — pulled fresh for this
/// session — else the embedded default
/// [`DEFAULT_TIE_OFF_CONTENT_INSTRUCTION`]), and a closing `---`.
///
/// **Never empty** — the tie-off concern is not scoped to producer
/// status: even a knot with no subscribers receives the contract and
/// the (possibly per-rig-overridden) tie-off content.
pub fn build_tie_off_context(tie_off_content_instruction: Option<&str>) -> String {
    let mut output = String::from("# Tie-Off\n\n");

    // The hard-coded universal contract — never overridable per rig.
    output.push_str(DEFAULT_TIE_OFF_CONTRACT);
    if !output.ends_with('\n') {
        output.push('\n');
    }

    // The overridable tie-off content section (doc 1) — pulled fresh
    // per session by the application layer and passed in.
    // `None` → the embedded default section.
    let content_section = tie_off_content_instruction
        .unwrap_or(DEFAULT_TIE_OFF_CONTENT_INSTRUCTION);
    output.push('\n');
    output.push_str(content_section);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("---\n\n");

    output
}

/// Build the listener context block to inject at the start of a target
/// knot's prompt.
///
/// Delegates to [`build_listener_context_with_instructions`] with the
/// embedded default event-content guide (no per-session source).
pub fn build_listener_context(
    knot: &Knot,
    loom_id: &LoomId,
    all_knots: &[Knot],
    self_continuation_desc: Option<&str>,
) -> String {
    build_listener_context_with_instructions(
        knot,
        loom_id,
        all_knots,
        self_continuation_desc,
        None,
    )
}

/// Same as [`build_listener_context`], but the **event-content guide**
/// (doc 2) is taken from `event_content_instruction` when `Some` —
/// content pulled fresh for this session — with the embedded default
/// as the `None` fallback. The hard-coded event core
/// ([`DEFAULT_EVENT_CONTRACT`]: acknowledgement, event format, rules)
/// is always injected when the block is built — never replaced.
///
/// Scans all knots' `strand_source` entries for `EventUri` subscriptions
/// where the current knot is the producer. Groups by `event-id` — if
/// multiple consumers listen for the same event from the same knot, they
/// are merged into one event block (not duplicated).
///
/// Uses the `event_description` from the first consumer knot declaring
/// each event. When `event_description` is absent, a generic message
/// is injected.
///
/// Returns an empty string when no consumers are listening (no injection
/// needed — the `# Subscriber Events` block is consumer-gated; the
/// `# Tie-Off` block is built separately and unconditionally).
///
/// The returned markdown is designed to be prepended to the knot's
/// instructions before execution.
pub fn build_listener_context_with_instructions(
    knot: &Knot,
    loom_id: &LoomId,
    all_knots: &[Knot],
    self_continuation_desc: Option<&str>,
    event_content_instruction: Option<&str>,
) -> String {
    use crate::domain::value_objects::StrandSource;

    // Collect all event subscriptions where this knot is the producer.
    // Matches both knot-level (producer_knot == knot.id) and
    // loom-level (producer_knot ends with "-loom" && == loom_id).
    let mut matching_knots: Vec<&Knot> = Vec::new();
    for other in all_knots {
        if let StrandSource::EventUri {
            producer_knot,
            ..
        } = &other.strand_source
        {
            let is_knot_level = producer_knot == &knot.id.0;
            let is_loom_level =
                producer_knot.ends_with("-loom") && producer_knot == &loom_id.0;
            if is_knot_level || is_loom_level {
                matching_knots.push(other);
            }
        }
    }

    // No listeners and no self-continuation — no injection needed.
    if matching_knots.is_empty() && self_continuation_desc.is_none() {
        return String::new();
    }

    // Group by event-id, preserving insertion order (first seen wins for
    // the description). Map event_id -> description.
    //
    // Plan 103 fix: this was a HashMap iterated directly — HashMap
    // iteration order is randomized per process, so the bullet run in
    // the emitted block was non-deterministic (the plan 102
    // byte-identical test flaked ~1-in-4 runs). A Vec keeps the
    // documented insertion-order contract.
    let mut seen_ids: Vec<(String, String)> = Vec::new();
    for consumer in &matching_knots {
        if let StrandSource::EventUri {
            producer_knot: _,
            event_id,
        } = &consumer.strand_source
        {
            if !seen_ids.iter().any(|(id, _)| id == event_id) {
                let desc = consumer.event_description.as_deref().unwrap_or(
                    "If this event occurs, emit a structured event block in your final response.",
                );
                seen_ids.push((event_id.clone(), desc.to_string()));
            }
        }
    }

    // Plan 086: add the implicit self-continuation entry for
    // water-marked aliases. The knot self-consumes this event; the
    // description is the handoff contract (TASKS_INCOMPLETE_DESCRIPTION).
    if let Some(desc) = self_continuation_desc {
        if !seen_ids.iter().any(|(id, _)| id == "TasksIncomplete") {
            seen_ids.push(("TasksIncomplete".to_string(), desc.to_string()));
        }
    }

    let mut output = String::from(
        "# Subscriber Events\n\n\
         The following event/s have been declared by subscribers:\n\n",
    );

    for (event_id, description) in &seen_ids {
        output.push_str(&format!(
            "- `{}` — {}\n",
            event_id, description
        ));
    }

    // The hard-coded event core — the subscriber acknowledgement, the
    // event format, and the rules — is always injected when the block
    // is built: it is the Knot agent's contract and is never
    // overridable per rig.
    output.push('\n');
    output.push_str(DEFAULT_EVENT_CONTRACT);
    if !output.ends_with('\n') {
        output.push('\n');
    }

    // The overridable event-content guide (doc 2) — pulled fresh per
    // session by the application layer and passed in. `None` → the
    // embedded default guide.
    let event_content_section = event_content_instruction
        .unwrap_or(DEFAULT_EVENT_CONTENT_INSTRUCTION);
    output.push('\n');
    output.push_str(event_content_section);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("---\n\n");

    output
}

// ── Domain Events ──────────────────────────────────────────────────────────

/// An event that describes the lifecycle of a Strand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// A producer's event emission was delivered to this knot (plan 099).
    ///
    /// Not a filesystem change: the message arrives embedded in the
    /// queue entry ([`EventDelivery`](crate::domain::pending_event::EventDelivery))
    /// and `strand_path` is the synthetic
    /// [`event_strand_descriptor`](crate::domain::pending_event::event_strand_descriptor)
    /// — there is no file to read. The consumer is given the rendered
    /// delivery in its prompt instead of an `@file` reference.
    EventDelivered {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        delivery: Box<crate::domain::pending_event::EventDelivery>,
    },
}

impl StrandEvent {
    /// Extract the strand path from any variant.
    pub fn strand_path(&self) -> &StrandPath {
        match self {
            StrandEvent::Created { strand_path, .. }
            | StrandEvent::Modified { strand_path, .. }
            | StrandEvent::Deleted { strand_path, .. }
            | StrandEvent::EventDelivered { strand_path, .. } => strand_path,
        }
    }

    /// The embedded delivery payload, when this is an event delivery.
    pub fn delivery(
        &self,
    ) -> Option<&crate::domain::pending_event::EventDelivery> {
        match self {
            StrandEvent::EventDelivered { delivery, .. } => Some(delivery),
            _ => None,
        }
    }

    /// Whether this event is a delivered emission rather than a
    /// filesystem change.
    pub fn is_event_delivery(&self) -> bool {
        matches!(self, StrandEvent::EventDelivered { .. })
    }
}

/// A TieOff (output file) was successfully produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TieOffProduced {
    pub knot_id: KnotId,
    pub strand_path: StrandPath,
    pub tie_off_path: TieOffPath,
}

/// Processing of a strand failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessingFailed {
    pub knot_id: KnotId,
    pub strand_path: StrandPath,
    pub error_message: String,
}

/// Plan 099 (Phase 3): the trace fields that ride a knot run.
///
/// These are the fields that make the debug trace chain durable and
/// greppable: rendered as structured `key=value` fields on the
/// run-lifecycle service-log lines, and as git commit trailers on the
/// knot's commit. A field that is `None` is **omitted** everywhere —
/// never written empty.
///
/// The values arrive from the delivery for an `EventDelivered` strand
/// (plan 099 Phase 1/2) and from the run context — the strand file's
/// front-matter and the captured session — for a file-triggered strand.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTrace {
    /// The pi session id of the run (known once the agent has started;
    /// `None` before execution and when the adapter reports no session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The event id that triggered the run (delivered emission id, or
    /// the strand file's `event-id` front-matter when it carries one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// The knot that emitted the triggering event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_knot: Option<String>,
    /// The strand whose change started the chain that produced this run.
    /// `None` when the chain has no file origin (a loom- or rig-scoped
    /// system event).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_strand: Option<String>,
}

impl RunTrace {
    /// True when no trace field is present.
    pub fn is_empty(&self) -> bool {
        self.session_id.is_none()
            && self.event_id.is_none()
            && self.source_knot.is_none()
            && self.original_strand.is_none()
    }
}

/// An event that describes the lifecycle of a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoomEvent {
    /// A new Knot was registered with the Loom.
    KnotRegistered {
        loom_id: LoomId,
        knot_id: KnotId,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// The Loom began processing its strands.
    LoomStarted {
        loom_id: LoomId,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// The Loom stopped processing.
    LoomStopped {
        loom_id: LoomId,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A strand was processed (either produced output or failed).
    StrandProcessed {
        loom_id: LoomId,
        strand_path: StrandPath,
        /// Error message if processing failed. `None` on success.
        error: Option<String>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A knot started processing a strand.
    KnotProcessing {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// Plan 099: the run's trace origin (event/source/original-strand;
        /// the session is not yet known at start). `None` when no trace
        /// is available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace: Option<RunTrace>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A knot completed processing a strand successfully.
    KnotCompleted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        tie_off_path: TieOffPath,
        /// Plan 099: the run's trace (incl. the captured session).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace: Option<RunTrace>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A knot failed while processing a strand.
    KnotFailed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        error: String,
        /// Plan 099: the run's trace (incl. the session, when the
        /// failure carried one).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace: Option<RunTrace>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A knot was deregistered from the loom.
    KnotDeregistered {
        loom_id: LoomId,
        knot_id: KnotId,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A knot file contained unknown YAML properties (accepted but not used).
    KnotParseWarning {
        loom_id: LoomId,
        knot_file_name: String,
        message: String,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// The strand directory for a knot was auto-created.
    DirectoryCreated {
        loom_id: LoomId,
        knot_id: KnotId,
        /// Absolute path of the directory that was created.
        directory: String,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A strand file was ignored (not a text file).
    ///
    /// Binary or non-text files in a strand directory are silently
    /// skipped. A warning is emitted on stderr (the event is also
    /// recorded in run activity).
    StrandIgnored {
        loom_id: LoomId,
        knot_id: KnotId,
        /// Path to the file that was ignored.
        strand_path: StrandPath,
        /// Reason the file was ignored (e.g. "binary file").
        reason: String,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A strand file was skipped because it could not be found on disk.
    ///
    /// Unlike [`StrandIgnored`] (binary files), this records a file that
    /// existed at event time but disappeared before processing — typically a
    /// short-lived temp file from editors like `sed -i`. Known temp-file
    /// patterns are silently dropped elsewhere; this variant logs the
    /// remaining unknown-missing-file cases so the user can investigate.
    StrandSkipped {
        loom_id: LoomId,
        knot_id: KnotId,
        /// Path to the file that was skipped.
        strand_path: StrandPath,
        /// Reason the file was skipped (e.g. "missing file (unknown pattern)").
        reason: String,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A failed agent invocation was resumed using the same Pi session.
    ///
    /// Recorded when a resumable error (timeout, mid-stream failure) is
    /// detected and Knot retries the invocation with `--session-id <id>`
    /// to continue the Pi session from where it left off.
    SessionResumed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// The agent completed within its timeout but produced no response.
    ///
    /// Recorded each time an invocation returns exit-code 0 with empty
    /// stdout — the agent session ended early (e.g. provider returned
    /// immediately) without generating any output. Logged per-attempt
    /// so the user can see repeated empty responses during retries.
    KnotEmptyResponse {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// Number of the attempt that produced the empty response
        /// (1 = first attempt, 2 = first retry, etc.).
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A compaction span has begun — pi's `compaction_start` observed
    /// **live** in the agent's JSON stream (plan 088). One entry per
    /// `compaction_start`, written when the line is observed (a long
    /// run that compacts several times shows each span as it happens).
    /// The matching end is `ContextCompacted` (success) or
    /// `ContextCompactionFailed` (failure / abort).
    CompactionStarted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// pi's compaction reason (`"threshold"` / `"overflow"` /
        /// `"manual"`).
        reason: String,
        /// Attempt the span began on
        /// (1 = first attempt, 2 = first retry, …).
        attempt: u32,
        timestamp: String,
    },
    /// The agent session's context hit (or approached) the model window
    /// and pi compacted it **successfully** — one entry per successful
    /// `compaction_end` observed live in the invocation's JSON stream
    /// (plan 088: written when the `compaction_end` line is observed,
    /// not after the invocation). `reason` is `"overflow"` (the context
    /// limit was hit — compacted to continue) or `"threshold"` (pi
    /// proactively compacted before the limit). The entry marks context
    /// pressure so the prompt/strand scope can be narrowed. This is the
    /// operator-facing context-pressure signal (troubleshooting:
    /// check `ContextCompacted` entries; `reason: "overflow"` = limit
    /// hit) — shape unchanged by plan 088; only the timing moved.
    /// Failed / aborted compactions are `ContextCompactionFailed`.
    ContextCompacted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        reason: String,
        tokens_before: Option<u64>,
        /// Attempt the compaction was observed on
        /// (1 = first attempt, 2 = first retry, …).
        attempt: u32,
        timestamp: String,
    },
    /// A compaction span ended **without success** — pi's
    /// `compaction_end` with an error or an abort, observed live in the
    /// agent's JSON stream (plan 088). Plan 079's `error.is_none()`
    /// filter (failed compactions were captured in metadata but not
    /// logged) is now a routing decision: failed ends surface here with
    /// pi's `errorMessage`, instead of being silent.
    ContextCompactionFailed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// pi's compaction reason (`"threshold"` / `"overflow"` /
        /// `"manual"`).
        reason: String,
        /// pi's `errorMessage` (`None` when the compaction was aborted
        /// without an error message).
        error: Option<String>,
        /// True when pi reported `aborted: true` (started but aborted,
        /// e.g. user interrupt / inactivity) rather than failed with an
        /// error message.
        aborted: bool,
        /// Attempt the end was observed on
        /// (1 = first attempt, 2 = first retry, …).
        attempt: u32,
        timestamp: String,
    },
    /// The `pi-rpc` adapter steered the agent to wrap up gracefully before
    /// its session context exhausted the model window. Plan 084 "Graceful
    /// Completion": the RPC runner samples `get_session_stats` on each
    /// `turn_end`; when `data.contextUsage.tokens` first crosses
    /// `ctx-wrap-up-limit` it sends a `steer` command with a wrap-up
    /// instruction. The adapter records at most one wrap-up per
    /// invocation (a `WrapUpRecord` on the invocation metadata), so this
    /// event fires at most once per successful invocation.
    ContextWrapUpSteered {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// The `data.contextUsage.tokens` value from the stats sample that
        /// tripped the steer.
        context_tokens: u64,
        /// The configured `ctx-wrap-up-limit` that was crossed.
        limit: u64,
        /// Attempt the steer was sent on
        /// (1 = first attempt, 2 = first retry, …).
        attempt: u32,
        /// How the handoff note was delivered (plan 086):
        /// `"steer"` (the `pi-rpc` live-steer path — the 084 default)
        /// or `"stop-resume"` (the `pi-json` SIGINT + re-invoke path).
        /// Serde-defaulted to `"steer"` so 084's pi-rpc records are
        /// unchanged on deserialization.
        #[serde(default = "default_wrap_up_mechanism")]
        mechanism: String,
        timestamp: String,
    },
    /// One or more agent events were dispatched to consumer knots.
    ///
    /// Recorded after a knot completes successfully and structured agent
    /// events are extracted from its tie-off. Lists which event-ids were
    /// dispatched, to which consumer looms, and the file each dispatch
    /// created — so a same-second fan-out can be traced file-by-file.
    ///
    /// The 4th tuple element (created file path) was added in Knot 0.33.0;
    /// legacy 3-tuple entries from earlier binaries fail to deserialize
    /// and are skipped by the activity reader with a warning (graceful
    /// degradation — no data loss, the producer tie-off retains all
    /// events). Same precedent as the 2→3 tuple expansion in 0.30.1.
    EventsDispatched {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// List of (event-id, consumer-knot-id, consumer-loom-id,
        /// created-file-path) quadruples dispatched. The path is absolute
        /// (the path the dispatcher returned when it created the file).
        dispatches: Vec<(String, String, String, String)>,
        /// Queue-busy observations (plan 097 phase 4): per dispatched
        /// event type, the number of same-type entries the shared queue
        /// net-gained during the session ((event-id, entries-gained),
        /// sorted by event id). Empty when the queue did not grow — the
        /// rendered log line then stays byte-identical to the
        /// pre-0.61.0 format. Legacy entries (pre-0.61.0 binaries)
        /// deserialize as empty (the field is skipped in their JSON).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        queue_busy: Vec<(String, u32)>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A knot completed successfully but was instructed to emit events
    /// and did not acknowledge all of them in its response.
    KnotEventsMissing {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// All events the agent was expected to acknowledge (the
        /// injected subscriber list, minus the `TasksIncomplete`
        /// self-continuation entry).
        expected_events: Vec<String>,
        /// The subset with no structured block in the tie-off (plan
        /// 091). Equals `expected_events` in the zero-block case.
        missing_events: Vec<String>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// The agent session produced no output for the inactivity window
    /// and was killed by the watchdog; the session is being restarted
    /// with the blocking-call note (plan 081). Logged per stall so the
    /// user can see repeated stalls during retries.
    AgentInactivity {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// The captured session ID; `""` when none was captured
        /// (pre-session stall or the `pi-stdio` adapter) — the restart
        /// is then a fresh session.
        session_id: String,
        /// How long the session was silent (seconds).
        silent_secs: u64,
        /// The configured inactivity window (seconds).
        window_secs: u64,
        /// The blocked call, when derivable from the stream
        /// (e.g. `bash("npm run build")`).
        blocked_call: Option<String>,
        /// Number of the attempt that stalled
        /// (1 = first attempt, 2 = first retry, etc.).
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A task-bearing session declared `TasksIncomplete: true` in its
    /// tie-off — work remains and the batch will continue via a
    /// self-continuation dispatch (plan 086). Recorded alongside the
    /// `KnotCompleted` tie-off event.
    ///
    /// `reason` ties the declaration back to the injection:
    /// `"water-mark"` — a `ContextWrapUpSteered` fired this session
    /// (the note asked the agent to wrap up); `"voluntary"` — the agent
    /// declared `occurred: true` at a stop with no water-mark.
    TasksIncomplete {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: Option<String>,
        /// Continuations carried by the dispatched event (1 = first hop).
        continuations: u32,
        /// Optional visibility-only progress (agent-populated; Knot
        /// never validates).
        tasks_done: Option<u32>,
        tasks_remaining: Option<u32>,
        /// Plan 087: the stamped `budget-secs` — the batch's remaining
        /// *execution* budget in seconds after this hop (queue wait is
        /// exempt; only execution decrements it).
        budget_secs: Option<u64>,
        /// Plan 087: the batch's `batch-start-epoch` (Unix epoch
        /// seconds — the batch's first handoff). Observability + the
        /// staleness backstop; never eroded by queue wait.
        batch_start_epoch: Option<u64>,
        /// Why the declaration was made.
        reason: String,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// A continuation chain stopped with work remaining (plan 086).
    ///
    /// `reason` is `"deadline"` (the batch's execution budget was
    /// exhausted before the continuation could spawn — no session,
    /// degenerate tie-off written) or `"caps"` (`continuations >=
    /// MAX_CONTINUATIONS` — dispatch suppressed). In both cases the work
    /// is not lost (checklist + commits are durable) and the batch
    /// resumes on the next dispatch with a fresh budget.
    BatchIncomplete {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// `"deadline"` or `"caps"`.
        reason: String,
        continuations: u32,
        /// Plan 087: the batch's remaining *execution* budget in seconds
        /// at the stop (the exhausted budget for `deadline`; the budget
        /// the suppressed continuation would have carried for `caps`).
        budget_secs: Option<u64>,
        /// Plan 087: the batch's `batch-start-epoch` (Unix epoch
        /// seconds — the batch's first handoff).
        batch_start_epoch: Option<u64>,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// Plan 089: pi's in-process auto-compaction started but the pi process
    /// stopped before it completed (an `overflow` `compaction_start` with no
    /// following `compaction_end`). The boundary of the recovery — Knot is
    /// about to attempt an out-of-band manual compact on the same session.
    /// Emitted by the usecase at the intervention boundary (not by the live
    /// stream observer that emits `CompactionStarted`).
    CompactionInterrupted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// The captured session id (always present — the recovery requires
        /// a session to open via `--session-id`).
        session_id: String,
        /// pi's compaction reason (the interrupted start's reason).
        reason: String,
        /// The attempt the interruption was detected on.
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// Plan 089: the out-of-band manual `compact` on an interrupted session
    /// succeeded — the context was shrunk below the model window, and the
    /// session is about to be re-entered (`SessionRestarted`).
    ManualCompactionSucceeded {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// Context tokens before the compact
        /// (`compaction_end.result.tokensBefore`).
        tokens_before: u64,
        /// The attempt the compact was run on.
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// Plan 089: the out-of-band manual `compact` on an interrupted session
    /// could not reduce the context — the session is over-full even after an
    /// explicit compact, so the strand fails (no re-entry).
    ManualCompactionFailed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// The failure reason (pi's `errorMessage`, a timeout, or an abort).
        error: String,
        /// The attempt the compact was attempted on.
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// Plan 089: the post-compact re-entry was attempted — the session was
    /// re-opened via `--session-id` with a "please continue" prompt after a
    /// successful manual compact. The run's own outcome (`KnotCompleted` /
    /// `KnotFailed`) follows.
    SessionRestarted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// The attempt the re-entry was made on.
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// Plan 089 (D6): Knot asked the compacted session to continue **in the
    /// same process** — a `prompt` on the still-open `pi-rpc` channel, sent
    /// when a compaction ended the turn with no final answer. The in-session
    /// sibling of `SessionRestarted` (which starts a *new* process via
    /// `--session-id`); it is the cheap path, so it is the one that runs on
    /// a healthy threshold compaction.
    TurnContinued {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        /// The compaction reason whose turn ended without an answer.
        reason: String,
        /// The attempt the continuation was sent on.
        attempt: u32,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// Plan 089 (D9): a queue entry survived the service — the run that
    /// picked it up never finished (Knot was stopped, crashed, or killed
    /// mid-strand), so its work is **abandoned**, not paused. Startup
    /// reports one of these per restored entry, before the queue is
    /// drained. The strand is then processed **from scratch** (a fresh
    /// `KnotProcessing` on a new session): Knot does not resume a dead
    /// agent session, and an operator comparing the `RunAbandoned` time
    /// with the next `KnotProcessing` should expect duplicated work, not
    /// continuation. `session_id` is `None` because the abandoned session
    /// id is never persisted — it died with the process.
    RunAbandoned {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: Option<String>,
        /// ISO 8601 timestamp (local time) — the restart time, not the
        /// time the run died.
        timestamp: String,
    },
}

/// Serde default for [`LoomEvent::ContextWrapUpSteered::mechanism`]:
/// `"steer"` (the 084 pi-rpc path).
fn default_wrap_up_mechanism() -> String {
    "steer".to_string()
}

/// A Knot was registered with a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnotRegistered {
    pub loom_id: LoomId,
    pub knot_id: KnotId,
}

// ── Rig-Log Events ─────────────────────────────────────────────────────────

/// An operational rig-level event (plan 083: run activity).
///
/// These serious operational events (timeouts, queue idle) are
/// in-memory run activity; each is rendered as a single-line
/// `[KNOT][EVENT]` record on stderr (the service log) so the user or
/// an external watcher can monitor and react.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RigLogEvent {
    /// An agent session exceeded its timeout deadline.
    TimeoutExceeded {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        error: String,
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
    /// All pending events have been processed and the queue is idle.
    QueueIdle {
        /// ISO 8601 timestamp (local time).
        timestamp: String,
    },
}

// ── Configuration Events ───────────────────────────────────────────────────

/// An event that describes configuration changes to looms and knots.
///
/// Unlike [`StrandEvent`] which tracks input file lifecycle, config events
/// track changes to the loom/knot definition files themselves (the `.md` knot
/// files and `*-loom` directories).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    use crate::domain::value_objects::StrandSource;
    use std::path::PathBuf;

    use crate::application::usecases::test_fixtures::KnotBuilder;

    // ── build_listener_context Tests (Phase 2) ────────────────────────

    fn default_loom_id() -> LoomId {
        LoomId("test-loom".to_string())
    }

    fn make_test_knot(id: &str) -> Knot {
        KnotBuilder::new(id)
            .with_instructions("test")
            .build()
    }

    fn make_event_knot(
        id: &str,
        producer_knot: &str,
        event_id: &str,
        event_description: Option<String>,
    ) -> Knot {
        KnotBuilder::new(id)
            .with_instructions("test")
            .with_strand_source(StrandSource::EventUri {
                producer_knot: producer_knot.to_string(),
                event_id: event_id.to_string(),
            })
            .with_event_description(event_description)
            .build()
    }

    /// No consumers listening for events — returns empty string.
    #[test]
    fn build_listener_context_no_consumers_returns_empty() {
        let producer = make_test_knot("plan-creator");
        let context = build_listener_context(&producer, &default_loom_id(), &[], None);
        assert!(
            context.is_empty(),
            "no consumers should produce empty context: '{}'",
            context
        );
    }

    /// No consumers listening — only filesystem knots — returns empty.
    #[test]
    fn build_listener_context_only_filesystem_knots_returns_empty() {
        let producer = make_test_knot("plan-creator");
        let filesystem_knot = make_test_knot("reviewer");
        let context = build_listener_context(&producer, &default_loom_id(), &[filesystem_knot], None);
        assert!(
            context.is_empty(),
            "only filesystem knots should produce empty context: '{}'",
            context
        );
    }

    /// Output starts with the `# Subscriber Events` heading.
    #[test]
    fn build_listener_context_output_has_heading() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.starts_with("# Subscriber Events\n"),
            "context should start with heading: {}",
            context
        );
    }

    /// Output contains the event description from the consumer knot.
    #[test]
    fn build_listener_context_output_contains_event_description() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created for the first time".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("When a plan is created for the first time"),
            "context should contain event description: {}",
            context
        );
    }

    /// Output does NOT contain consumer knot names in the event list.
    /// Only the event ID and description are visible to the producer.
    #[test]
    fn build_listener_context_output_does_not_contain_consumer_knot_names() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "secret-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        // The consumer knot ID should NOT appear in the output
        assert!(
            !context.contains("secret-validator"),
            "context should NOT contain consumer knot name: {}",
            context
        );
    }

    /// Output contains instructions for `occurred` field.
    #[test]
    fn build_listener_context_output_instructs_occurred_field() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("occurred:"),
            "context should instruct to use 'occurred' field: {}",
            context
        );
        assert!(
            context.contains("occurred: true"),
            "context should show 'occurred: true' in example: {}",
            context
        );
    }

    /// Output instructs the producer to include a `description` field.
    #[test]
    fn build_listener_context_output_requires_description_field() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("description:"),
            "context should require description field: {}",
            context
        );
    }

    /// Single consumer triggers context with its event description.
    #[test]
    fn build_listener_context_single_consumer_triggers_context() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(!context.is_empty());
        assert!(context.contains("# Subscriber Events"));
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("When a plan is created"));
        assert!(context.contains("occurred:"));
        assert!(context.contains("description:"));
    }

    /// Multiple consumers listening for the same event deduplicate
    /// — only one entry appears in the output.
    #[test]
    fn build_listener_context_multiple_consumers_same_event_deduplicates() {
        let producer = make_test_knot("plan-creator");
        let consumer1 = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let consumer2 = make_event_knot(
            "plan-auditor",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created for audit".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer1, consumer2], None);
        // Count occurrences of "PlanCreated" in the event list (should appear
        // only once as a bullet point)
        let count = context.matches("- `PlanCreated`").count();
        assert_eq!(count, 1, "same event from multiple consumers should deduplicate: {}", context);
    }

    /// Multiple different events from the same producer each appear.
    #[test]
    fn build_listener_context_multiple_different_events() {
        let producer = make_test_knot("plan-creator");
        let consumer1 = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let consumer2 = make_event_knot(
            "plan-fixer",
            "plan-creator",
            "ValidationFailed",
            Some("When validation fails".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer1, consumer2], None);
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("ValidationFailed"));
        assert!(context.contains("When a plan is created"));
        assert!(context.contains("When validation fails"));
    }

    /// When event-description is None, a generic message is used.
    #[test]
    fn build_listener_context_generic_message_when_event_description_none() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            None, // no event-description
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("If this event occurs, emit a structured event block in your final response."),
            "should use generic message when event-description is None: {}",
            context
        );
        // Consumer knot name should not appear even in generic message
        assert!(
            !context.contains("plan-validator"),
            "generic message should not contain consumer knot name: {}",
            context
        );
    }

    /// The injected universal tie-off contract (the unconditional
    /// `# Tie-Off` block) tells the agent never to access tie-off files
    /// directly — they are owned by the infrastructure.
    #[test]
    fn tie_off_context_forbids_direct_tieoff_access() {
        let context = build_tie_off_context(None);
        assert!(
            context.contains("never write or read directly from"),
            "tie-off directions should forbid direct tie-offs/ access: {}",
            context
        );
        assert!(
            context.contains("`tie-offs/`"),
            "tie-off directions must name the tie-offs/ dir: {}",
            context
        );
        assert!(
            context.contains("infrastructure service"),
            "tie-off directions should state tie-offs are infrastructure-managed: {}",
            context
        );
    }

    /// Plan 102: **leaf knot** (no consumers, no self-continuation) —
    /// the `# Subscriber Events` block is consumer-gated (empty), but
    /// the `# Tie-Off` block is unconditional: the full block with both
    /// default sections (the universal contract + the default
    /// tie-off content).
    #[test]
    fn tie_off_block_is_unconditional_for_leaf_knots() {
        let leaf = make_test_knot("orchestrator");
        let all_knots: [Knot; 0] = [];
        let listener = build_listener_context(&leaf, &default_loom_id(), &all_knots, None);
        assert!(
            listener.is_empty(),
            "leaf knot: no subscriber events block: '{}'",
            listener
        );
        let tie_off = build_tie_off_context(None);
        assert!(
            tie_off.starts_with("# Tie-Off\n\n"),
            "tie-off block must start with the # Tie-Off header: {}",
            tie_off
        );
        assert!(
            tie_off.contains("## Tie-Off Contract"),
            "hard-coded universal contract must be present: {}",
            tie_off
        );
        assert!(
            tie_off.contains("never write or read directly from"),
            "universal contract sentence must reach leaf knots: {}",
            tie_off
        );
        assert!(
            tie_off.contains("`tie-offs/`"),
            "universal contract must name the tie-offs/ dir: {}",
            tie_off
        );
        assert!(
            tie_off.contains("## Tie-Off Content"),
            "default tie-off content section must be present: {}",
            tie_off
        );
        assert!(
            !tie_off.contains("# Subscriber Events"),
            "leaf knot: no subscriber events block in the tie-off: {}",
            tie_off
        );
        assert!(
            !tie_off.contains("## Event Content Guide"),
            "leaf knot: no event-content guide: {}",
            tie_off
        );
        assert!(
            tie_off.trim_end().ends_with("---"),
            "tie-off block must close with ---: {}",
            tie_off
        );
    }

    /// Plan 102: **consumer knot** — both blocks are present, the
    /// `# Tie-Off` block first, then `# Subscriber Events`; the
    /// `## Tie-Off Contract` heading appears exactly once per prompt.
    #[test]
    fn consumer_knot_prompt_has_both_blocks_in_order() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let tie_off = build_tie_off_context(None);
        let listener = build_listener_context(
            &producer,
            &default_loom_id(),
            &[consumer],
            None,
        );
        assert!(!listener.is_empty());
        let prompt = format!("{}{}", tie_off, listener);
        let tie_off_pos = prompt.find("# Tie-Off").unwrap();
        let listener_pos = prompt.find("# Subscriber Events").unwrap();
        assert!(
            tie_off_pos < listener_pos,
            "# Tie-Off must precede # Subscriber Events: {}",
            prompt
        );
        assert_eq!(
            prompt.matches("## Tie-Off Contract").count(),
            1,
            "the ## Tie-Off Contract heading must appear exactly once: {}",
            prompt
        );
    }

    /// Plan 102: the marker line and the contiguous `- \`EventId\``
    /// bullet run are byte-identical to the pre-split shape (protects
    /// `extract_emitted_event_ids`, which anchors on them).
    #[test]
    fn listener_block_marker_and_bullets_are_byte_identical() {
        let producer = make_test_knot("plan-creator");
        let consumer1 = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let consumer2 = make_event_knot(
            "plan-fixer",
            "plan-creator",
            "ValidationFailed",
            Some("When validation fails".to_string()),
        );
        let listener = build_listener_context(
            &producer,
            &default_loom_id(),
            &[consumer1, consumer2],
            None,
        );
        assert!(
            listener.starts_with(
                "# Subscriber Events\n\n\
                 The following event/s have been declared by subscribers:\n\n\
                 - `PlanCreated` — When a plan is created\n\
                 - `ValidationFailed` — When validation fails\n"
            ),
            "marker line + bullet run must be byte-identical: {}",
            listener
        );
    }

    /// Plan 102: a `Some` doc-1 override replaces **only the
    /// tie-off content** — the default `## Event Content Guide` text
    /// stays in the (consumer knot's) listener block; the `## Tie-Off
    /// Contract` and the event format/rules cores are present.
    #[test]
    fn doc1_override_replaces_only_tie_off_content() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let tie_off = build_tie_off_context(Some("CUSTOM TIE-OFF CONTENT"));
        let listener = build_listener_context_with_instructions(
            &producer,
            &default_loom_id(),
            &[consumer],
            None,
            None,
        );
        let prompt = format!("{}{}", tie_off, listener);
        assert!(
            prompt.contains("CUSTOM TIE-OFF CONTENT"),
            "doc-1 override must be injected: {}",
            prompt
        );
        assert!(
            !prompt.contains("## Tie-Off Content"),
            "default tie-off content must be replaced, not appended: {}",
            prompt
        );
        // The other section is unaffected — the default event-content
        // guide is still present:
        assert!(
            prompt.contains("## Event Content Guide"),
            "default event-content guide must stay: {}",
            prompt
        );
        // The hard-coded cores are always present — never replaced:
        assert!(
            prompt.contains("## Tie-Off Contract"),
            "hard-coded universal contract must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("## Event Format"),
            "hard-coded event format must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("## Rules"),
            "hard-coded rules must stay: {}",
            prompt
        );
    }

    /// Plan 102: a `Some` doc-2 override replaces **only the
    /// event-content guide** — the default `## Tie-Off Content` section
    /// stays in the `# Tie-Off` block; the event cores are present.
    #[test]
    fn doc2_override_replaces_only_event_content_guide() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let listener = build_listener_context_with_instructions(
            &producer,
            &default_loom_id(),
            &[consumer],
            None,
            Some("CUSTOM EVENT CONTENT GUIDE"),
        );
        let tie_off = build_tie_off_context(None);
        let prompt = format!("{}{}", tie_off, listener);
        assert!(
            prompt.contains("CUSTOM EVENT CONTENT GUIDE"),
            "doc-2 override must be injected: {}",
            prompt
        );
        assert!(
            !prompt.contains("## Event Content Guide"),
            "default event-content guide must be replaced, not appended: {}",
            prompt
        );
        // The other section is unaffected — the default tie-off content
        // is still present:
        assert!(
            prompt.contains("## Tie-Off Content"),
            "default tie-off content must stay: {}",
            prompt
        );
        // The hard-coded event core is always present — never replaced:
        assert!(
            prompt.contains("## Event Acknowledgement"),
            "hard-coded event acknowledgement must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("## Event Format"),
            "hard-coded event format must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("## Rules"),
            "hard-coded rules must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("event: EventId"),
            "hard-coded event properties must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("occurred: true"),
            "hard-coded event properties must stay: {}",
            prompt
        );
        assert!(
            prompt.contains("Emit exactly one event block per subscriber event"),
            "hard-coded emit-always rule must stay: {}",
            prompt
        );
        // The subscriber bullets are untouched.
        assert!(prompt.contains("# Subscriber Events"));
        assert!(prompt.contains("`PlanCreated`"));
    }

    /// Plan 102: both `None` → full defaults — the `# Tie-Off` block
    /// with both default sections, then the `# Subscriber Events` block
    /// with the event cores and the default event-content guide.
    #[test]
    fn both_sections_default_when_no_instructions() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let tie_off = build_tie_off_context(None);
        let listener = build_listener_context_with_instructions(
            &producer,
            &default_loom_id(),
            &[consumer],
            None,
            None,
        );
        let prompt = format!("{}{}", tie_off, listener);
        assert!(prompt.contains("## Tie-Off Contract"));
        assert!(prompt.contains("## Tie-Off Content"));
        assert!(prompt.contains("## Event Acknowledgement"));
        assert!(prompt.contains("## Event Format"));
        assert!(prompt.contains("## Rules"));
        assert!(prompt.contains("## Event Content Guide"));
        assert!(prompt.contains("You must acknowledge each"));
        assert!(!prompt.contains("acknolowedge"));
    }

    /// Plan 102: the self-continuation (`TasksIncomplete`) entry still
    /// yields the `# Subscriber Events` block with its implicit
    /// consumer, even for a knot with no external subscribers.
    #[test]
    fn self_continuation_entry_yields_listener_block() {
        let producer = make_test_knot("plan-creator");
        let all_knots: [Knot; 0] = [];
        let listener = build_listener_context_with_instructions(
            &producer,
            &default_loom_id(),
            &all_knots,
            Some("handoff contract"),
            None,
        );
        assert!(
            !listener.is_empty(),
            "self-continuation must yield the listener block: {}",
            listener
        );
        assert!(
            listener.contains("- `TasksIncomplete` — handoff contract"),
            "the implicit self-continuation consumer must be listed: {}",
            listener
        );
        assert!(listener.contains("## Event Format"));
        assert!(listener.contains("## Event Content Guide"));
    }

    /// Plan 097: the default event-content guide carries the dedup norm
    /// — the per-rig instruction surface is where the queued-events
    /// dedup rule lives (the injected `## Queued Events` section points
    /// at it). The shipped template is the source of the embedded
    /// default (`include_str!`), so this conformance test pins both the
    /// norm and the pointer target.
    #[test]
    fn default_content_instruction_carries_dedup_norm() {
        assert!(
            DEFAULT_EVENT_CONTENT_INSTRUCTION.contains("Deduplication"),
            "the dedup norm must live in the event-content guide (rig wording)"
        );
        assert!(
            DEFAULT_EVENT_CONTENT_INSTRUCTION.contains("Queued Events"),
            "the norm must reference the section by its prompt heading"
        );
        // And it reaches the built prompt via the default path.
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context_with_instructions(
            &producer,
            &default_loom_id(),
            &[consumer],
            None,
            None,
        );
        assert!(context.contains("Deduplication"));
    }

    /// The wrapper (source-less entry point) is byte-identical to the
    /// with-instructions variant on `None`.
    #[test]
    fn listener_context_wrapper_matches_none_none_variant() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let knots = [consumer];
        let via_wrapper = build_listener_context(
            &producer,
            &default_loom_id(),
            &knots,
            None,
        );
        let via_variant = build_listener_context_with_instructions(
            &producer,
            &default_loom_id(),
            &knots,
            None,
            None,
        );
        assert_eq!(via_wrapper, via_variant);
    }

    /// Consumers for a different producer knot do not affect output.
    #[test]
    fn build_listener_context_other_producer_no_effect() {
        let producer = make_test_knot("plan-creator");
        let other_consumer = make_event_knot(
            "other-validator",
            "other-producer", // different producer
            "OtherEvent",
            Some("Some event".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[other_consumer], None);
        assert!(context.is_empty());
    }

    /// Mixed: one consumer for this producer, one for another.
    /// Only the matching consumer appears.
    #[test]
    fn build_listener_context_mixed_consumers_only_matching() {
        let producer = make_test_knot("plan-creator");
        let matching = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let non_matching = make_event_knot(
            "other-validator",
            "other-producer",
            "OtherEvent",
            Some("Some event".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[matching, non_matching], None);
        assert!(!context.is_empty());
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("When a plan is created"));
        assert!(!context.contains("OtherEvent"));
        assert!(!context.contains("other-validator"));
    }

    // ── Phase 3: New format tests ─────────────────────────

    /// Prompt uses ```markdown fence (not plain ```).
    #[test]
    fn build_listener_context_prompt_uses_markdown_fence() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("```markdown"),
            "prompt should use ```markdown fence: {}",
            context
        );
    }

    /// Prompt shows frontmatter (--- delimiters) and body structure.
    #[test]
    fn build_listener_context_prompt_shows_frontmatter_and_body() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("---"),
            "prompt should show frontmatter delimiters (---): {}",
            context
        );
        assert!(
            context.contains("narrative context"),
            "prompt should show body/narrative context area: {}",
            context
        );
    }

    /// Prompt instructs `occurred: false` as the way to signal no event.
    #[test]
    fn build_listener_context_occurred_false_instruction() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        // Prompt should NOT contain event: None anymore
        assert!(
            !context.contains("event: None"),
            "context should NOT contain 'event: None': {}",
            context
        );
        // Prompt should contain occurred: true in the example
        assert!(
            context.contains("occurred: true"),
            "context should show 'occurred: true' in example: {}",
            context
        );
        // Prompt should contain occurred: false in the rules
        assert!(
            context.contains("occurred: false"),
            "context should mention 'occurred: false' in rules: {}",
            context
        );
    }

    // ── Phase 3: Timestamp and required fields ─────────────────

    /// Prompt template includes `timestamp:` field in the format example.
    #[test]
    fn build_listener_context_prompt_includes_timestamp_field() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("timestamp:"),
            "prompt should include timestamp field: {}",
            context
        );
        assert!(
            context.contains("2026-"),
            "prompt should show a concrete ISO 8601 timestamp example: {}",
            context
        );
    }

    /// Prompt template includes "do not edit" guidance text.
    #[test]
    fn build_listener_context_prompt_includes_do_not_edit_guidance() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("do not edit the pending event"),
            "prompt should contain 'do not edit' guidance: {}",
            context
        );
        assert!(
            context.contains("emit a new event"),
            "prompt should instruct to emit new event if adjustment needed: {}",
            context
        );
    }

    /// Prompt template documents that `event`, `description`, and `timestamp`
    /// are required fields.
    #[test]
    fn build_listener_context_prompt_documents_required_fields() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &default_loom_id(), &[consumer], None);
        assert!(
            context.contains("required"),
            "prompt should mention required fields: {}",
            context
        );
        // The required fields mention should reference event, description,
        // and timestamp
        assert!(
            context.contains("event")
                && context.contains("description")
                && context.contains("timestamp"),
            "prompt should reference event, description, and timestamp as required: {}",
            context
        );
    }

    // ── Loom-Level Subscription Tests ────────────────────────────

    /// Consumer with `event:planning-loom:PlanCreated` matches a knot
    /// inside `planning-loom`.
    #[test]
    fn build_listener_context_loom_level_consumer_matches_producer_in_loom() {
        let producer = make_test_knot("plan-creator");
        let loom_id = LoomId("planning-loom".to_string());
        let consumer = make_event_knot(
            "plan-validator",
            "planning-loom", // loom-level subscription
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &loom_id, &[consumer], None);
        assert!(!context.is_empty());
        assert!(context.contains("# Subscriber Events"));
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("When a plan is created"));
    }

    /// Consumer with `event:planning-loom:PlanCreated` does NOT match
    /// a knot inside `review-loom`.
    #[test]
    fn build_listener_context_loom_level_consumer_no_match_different_loom() {
        let producer = make_test_knot("plan-creator");
        let loom_id = LoomId("review-loom".to_string());
        let consumer = make_event_knot(
            "plan-validator",
            "planning-loom", // subscribed to a different loom
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &loom_id, &[consumer], None);
        assert!(
            context.is_empty(),
            "loom-level subscription should not match a different loom: {}",
            context
        );
    }

    /// Both knot-level and loom-level consumers for the same event appear
    /// (deduplicated by event ID).
    #[test]
    fn build_listener_context_mixed_knot_and_loom_consumers() {
        let producer = make_test_knot("plan-creator");
        let loom_id = LoomId("planning-loom".to_string());
        // Knot-level consumer — targets this specific knot
        let knot_consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        // Loom-level consumer — targets the entire loom
        let loom_consumer = make_event_knot(
            "plan-auditor",
            "planning-loom",
            "PlanCreated",
            Some("When a plan is created for audit".to_string()),
        );
        let context =
            build_listener_context(&producer, &loom_id, &[knot_consumer, loom_consumer], None);
        assert!(!context.is_empty());
        assert!(context.contains("PlanCreated"));
        // Both consumers subscribe to the same event — should deduplicate
        let count = context.matches("- `PlanCreated`").count();
        assert_eq!(
            count, 1,
            "same event from knot-level and loom-level consumers should deduplicate: {}",
            context
        );
    }

    /// Verify that every knot in the subscribed-to loom receives event
    /// instructions.
    #[test]
    fn build_listener_context_all_knots_in_loom_get_injection() {
        let loom_id = LoomId("planning-loom".to_string());
        let consumer = make_event_knot(
            "plan-validator",
            "planning-loom", // loom-level subscription
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );

        // Multiple knots in the same loom — each should get injection
        let knot1 = make_test_knot("plan-creator");
        let knot2 = make_test_knot("plan-reviewer");
        let knot3 = make_test_knot("plan-approver");

        let ctx1 = build_listener_context(&knot1, &loom_id, &[consumer.clone()], None);
        let ctx2 = build_listener_context(&knot2, &loom_id, &[consumer.clone()], None);
        let ctx3 = build_listener_context(&knot3, &loom_id, &[consumer.clone()], None);

        assert!(!ctx1.is_empty(), "knot1 should get injection");
        assert!(!ctx2.is_empty(), "knot2 should get injection");
        assert!(!ctx3.is_empty(), "knot3 should get injection");
        assert!(ctx1.contains("PlanCreated"));
        assert!(ctx2.contains("PlanCreated"));
        assert!(ctx3.contains("PlanCreated"));
    }

    // ── AgentEvent Tests ─────────────────────────────────────────

    #[test]
    fn agent_event_construction() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        payload.insert(
            "description".to_string(),
            "Implementation plan".to_string(),
        );

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            occurred: true,
            payload,
            body: None,
        };

        assert_eq!(event.event_id, "PlanCreated");
        assert_eq!(event.occurred, true);
        assert_eq!(event.payload.len(), 2);
        assert_eq!(
            event.payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
        assert_eq!(event.body, None);
    }

    #[test]
    fn agent_event_serialisation_roundtrip() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-007".to_string());

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            occurred: true,
            payload,
            body: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn agent_event_empty_payload_defaults() {
        let event = AgentEvent {
            event_id: "Something".to_string(),
            occurred: true,
            payload: HashMap::new(),
            body: None,
        };

        // Serialize and deserialize — empty payload should survive
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert!(deserialized.payload.is_empty());
    }

    #[test]
    fn agent_event_missing_payload_in_json_defaults_to_empty() {
        // JSON without a payload or occurred field should deserialize
        // with defaults (empty HashMap, occurred=true)
        let json = r#"{"event_id":"Test"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_id, "Test");
        assert!(event.payload.is_empty());
        assert!(event.occurred, "occurred should default to true");
    }

    #[test]
    fn agent_event_with_body_roundtrips_through_json() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-010".to_string());

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            occurred: true,
            payload,
            body: Some(
                "The plan covers three phases: planning, review, and approval.".to_string(),
            ),
        };

        let json = serde_json::to_string(&event).unwrap();
        // Verify body appears in JSON
        assert!(
            json.contains("body"),
            "JSON should contain body field: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(
            deserialized.body.as_deref(),
            Some("The plan covers three phases: planning, review, and approval.")
        );
    }

    #[test]
    fn agent_event_with_none_body_survives_serialisation() {
        let event = AgentEvent {
            event_id: "NoBodyEvent".to_string(),
            occurred: true,
            payload: HashMap::new(),
            body: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        // body is skip_serializing_if = is_none, so it should not appear
        assert!(
            !json.contains("body"),
            "JSON should not contain body when None: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(deserialized.body, None);
    }

    #[test]
    fn agent_event_missing_body_in_json_defaults_to_none() {
        // JSON without a body field should deserialize with None
        let json = r#"{"event_id":"Test","payload":{"key":"val"}}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_id, "Test");
        assert_eq!(event.body, None);
        assert!(event.occurred, "occurred should default to true");
    }

    #[test]
    fn agent_event_with_empty_string_body_preserved() {
        let event = AgentEvent {
            event_id: "EmptyBody".to_string(),
            occurred: true,
            payload: HashMap::new(),
            body: Some(String::new()),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains("body"),
            "JSON should contain body even when empty string: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(deserialized.body.as_deref(), Some(""));
    }

    #[test]
    fn agent_event_occurred_false_roundtrips() {
        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            occurred: false,
            payload: HashMap::new(),
            body: None,
        };

        assert!(!event.occurred);
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains("\"occurred\":false"),
            "JSON should contain occurred:false: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert!(!deserialized.occurred);
    }

    #[test]
    fn agent_event_occurred_defaults_to_true_in_json() {
        // JSON without occurred field should deserialize with occurred=true
        let json = r#"{"event_id":"Test","payload":{}}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert!(event.occurred, "occurred should default to true");
    }

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
    fn loom_event_context_wrap_up_steered_roundtrip() {
        let event = LoomEvent::ContextWrapUpSteered {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            session_id: "sess-42".to_string(),
            context_tokens: 150_000,
            limit: 140_000,
            attempt: 1,
            mechanism: "steer".to_string(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        // Externally tagged with the variant name.
        assert!(json.contains("ContextWrapUpSteered"));
    }

    /// Plan 097 phase 4 — `EventsDispatched` serde: the `queue_busy`
    /// field round-trips, is omitted from JSON when empty (the log line
    /// stays format-stable), and legacy entries (pre-0.61.0, no field in
    /// the JSON) deserialize as empty.
    #[test]
    fn loom_event_events_dispatched_queue_busy_serde() {
        let ts = "2026-06-10T12:00:00Z".to_string();
        let event = LoomEvent::EventsDispatched {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("strands/spec.md")),
            dispatches: vec![(
                "PlanCreated".to_string(),
                "write".to_string(),
                "docs".to_string(),
                "/p/tie-offs/rig/docs-loom/spec.md".to_string(),
            )],
            queue_busy: vec![("PlanCreated".to_string(), 3)],
            timestamp: ts,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);

        // Empty queue_busy is omitted from the JSON (format-stable line).
        let quiet = LoomEvent::EventsDispatched {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("strands/spec.md")),
            dispatches: vec![],
            queue_busy: Vec::new(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&quiet).unwrap();
        assert!(!json.contains("queue_busy"));

        // Legacy JSON (pre-0.61.0 — no queue_busy field) still parses:
        // the field defaults to empty, so old activity entries survive
        // the binary upgrade (graceful degradation, same precedent as
        // the 3-tuple → 4-tuple dispatch expansion in 0.33.0).
        let legacy = r#"{"EventsDispatched":{"loom_id":"prds","knot_id":"review","strand_path":"strands/spec.md","dispatches":[["PlanCreated","write","docs","/p/x.md"]],"timestamp":"2026-06-10T12:00:00Z"}}"#;
        let deserialized: LoomEvent = serde_json::from_str(legacy).unwrap();
        match deserialized {
            LoomEvent::EventsDispatched { queue_busy, .. } => {
                assert!(queue_busy.is_empty());
            }
            other => panic!("expected EventsDispatched, got {other:?}"),
        }
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
            trace: None,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotProcessing {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                trace: _,
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
            trace: None,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotCompleted {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                tie_off_path: tp,
                trace: _,
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
            trace: None,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotFailed {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                error: msg,
                trace: _,
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
        KnotBuilder::new(id)
            .with_instructions("Test instructions.")
            .build()
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
                trace: None,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotCompleted {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                tie_off_path: tie_off_path.clone(),
                trace: None,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotFailed {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                error: "boom".to_string(),
                trace: None,
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
            LoomEvent::DirectoryCreated {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                directory: "/project/rig/prds-loom/strands".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::StrandIgnored {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                reason: "binary file".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::StrandSkipped {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                reason: "missing file (unknown pattern)".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::SessionResumed {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                session_id: "sess-abc123".to_string(),
                attempt: 2,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotEmptyResponse {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                attempt: 3,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotEventsMissing {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                expected_events: vec![
                    "PlanCreated".to_string(),
                    "ValidationFailed".to_string(),
                ],
                missing_events: vec!["ValidationFailed".to_string()],
                timestamp: ts.clone(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, *event, "round-trip failed for variant");
        }
    }

    /// `DirectoryCreated` carries `loom_id`, `knot_id`, `directory`, and
    /// `timestamp`. Verifies the variant shape and JSON round-trip.
    #[test]
    fn loom_event_directory_created_serialisation() {
        let loom_id = LoomId("auto-strand-dir-loom".to_string());
        let knot_id = KnotId("codegen".to_string());
        let directory = "/project/rig/auto-strand-dir-loom/strands".to_string();
        let ts = "2026-06-17T09:00:00Z".to_string();

        let event = LoomEvent::DirectoryCreated {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            directory: directory.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id: kid,
                directory: dir,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(dir, &directory);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected DirectoryCreated variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
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

    /// `LoomEvent::StrandIgnored` carries `loom_id`, `knot_id`,
    /// `strand_path`, `reason`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation.
    #[test]
    fn loom_event_strand_ignored() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/image.png"));
        let reason = "binary file".to_string();
        let ts = "2026-06-19T10:00:00Z".to_string();

        let event = LoomEvent::StrandIgnored {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            reason: reason.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::StrandIgnored {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                reason: r,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(r, &reason);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected StrandIgnored variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `LoomEvent::StrandSkipped` carries `loom_id`, `knot_id`,
    /// `strand_path`, `reason`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation.
    #[test]
    fn loom_event_strand_skipped() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/sedABC123"));
        let reason = "missing file (unknown pattern)".to_string();
        let ts = "2026-06-24T10:00:00Z".to_string();

        let event = LoomEvent::StrandSkipped {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            reason: reason.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::StrandSkipped {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                reason: r,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(r, &reason);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected StrandSkipped variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `LoomEvent::SessionResumed` carries `loom_id`, `knot_id`,
    /// `strand_path`, `session_id`, `attempt`, and `timestamp`.
    /// Verifies the variant shape and JSON round-trip serialisation.
    #[test]
    fn session_resumed_event_serialisation() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let session_id = "sess-resume-42".to_string();
        let attempt: u32 = 3;
        let ts = "2026-06-28T14:00:00Z".to_string();

        let event = LoomEvent::SessionResumed {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            session_id: session_id.clone(),
            attempt,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::SessionResumed {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                session_id: sid,
                attempt: a,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(sid, &session_id);
                assert_eq!(*a, attempt);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected SessionResumed variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `LoomEvent::KnotEmptyResponse` carries `loom_id`, `knot_id`,
    /// `strand_path`, `attempt`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation.
    #[test]
    fn loom_event_knot_empty_response() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let attempt: u32 = 2;
        let ts = "2026-06-28T15:00:00Z".to_string();

        let event = LoomEvent::KnotEmptyResponse {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            attempt,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotEmptyResponse {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                attempt: a,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(*a, attempt);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotEmptyResponse variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    // ── KnotEventsMissing Tests (Phase 0) ─────────────────────────

    /// `KnotEventsMissing` serialises and deserialises correctly,
    /// preserving all fields including the `expected_events` vec.
    #[test]
    fn knot_events_missing_event_serialisation() {
        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("plan-creator".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let ts = "2026-07-14T10:00:00Z".to_string();

        let event = LoomEvent::KnotEventsMissing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            expected_events: vec![
                "PlanCreated".to_string(),
                "ValidationFailed".to_string(),
            ],
            missing_events: vec!["ValidationFailed".to_string()],
            timestamp: ts.clone(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `KnotEventsMissing` preserves the `expected_events` vec through
    /// pattern matching and serialisation.
    #[test]
    fn knot_events_missing_event_fields() {
        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("plan-creator".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let ts = "2026-07-14T10:00:00Z".to_string();

        let event = LoomEvent::KnotEventsMissing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            expected_events: vec![
                "PlanCreated".to_string(),
                "ValidationFailed".to_string(),
            ],
            missing_events: vec!["ValidationFailed".to_string()],
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotEventsMissing {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                expected_events,
                missing_events,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(expected_events.len(), 2);
                assert_eq!(expected_events[0], "PlanCreated");
                assert_eq!(expected_events[1], "ValidationFailed");
                assert_eq!(
                    missing_events,
                    &vec!["ValidationFailed".to_string()]
                );
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotEventsMissing variant"),
        }
    }

    // ── AgentInactivity Tests (Plan 081) ─────────────────────────────

    /// `LoomEvent::AgentInactivity` carries `loom_id`, `knot_id`,
    /// `strand_path`, `session_id`, `silent_secs`, `window_secs`,
    /// `blocked_call`, `attempt`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation (field order stable —
    /// it is a new variant, no compatibility concern).
    #[test]
    fn agent_inactivity_event_serialisation() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let session_id = "sess-inact-42".to_string();
        let silent_secs: u64 = 300;
        let window_secs: u64 = 300;
        let blocked_call = Some("bash(\"npm run build\")".to_string());
        let attempt: u32 = 2;
        let ts = "2026-09-02T10:00:00Z".to_string();

        let event = LoomEvent::AgentInactivity {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            session_id: session_id.clone(),
            silent_secs,
            window_secs,
            blocked_call: blocked_call.clone(),
            attempt,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::AgentInactivity {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                session_id: sid,
                silent_secs: ss,
                window_secs: ws,
                blocked_call: bc,
                attempt: a,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(sid, &session_id);
                assert_eq!(*ss, silent_secs);
                assert_eq!(*ws, window_secs);
                assert_eq!(bc, &blocked_call);
                assert_eq!(*a, attempt);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected AgentInactivity variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// A stall with no captured session (pre-session line, `pi-stdio`)
    /// serialises with an empty `session_id` and no blocked call.
    #[test]
    fn agent_inactivity_event_without_session_roundtrips() {
        let event = LoomEvent::AgentInactivity {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            session_id: String::new(),
            silent_secs: 42,
            window_secs: 300,
            blocked_call: None,
            attempt: 1,
            timestamp: "2026-09-02T10:00:01Z".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    // ── Phase 0: ContextProvider and BuildContext Tests ──────────────

    /// `BuildContext` carries all required fields with correct values.
    #[test]
    fn build_context_carries_all_fields() {
        let knot = KnotBuilder::new("plan-creator")
            .with_instructions("create plans")
            .build();
        let loom_id = LoomId("planning-loom".to_string());
        let all_knots = vec![knot.clone()];
        let rig_dir = PathBuf::from("/tmp/rig");

        let ctx = BuildContext {
            knot: knot.clone(),
            loom_id: loom_id.clone(),
            all_knots: all_knots.clone(),
            rig_dir: rig_dir.clone(),
            strand_queue: None,
        self_continuation_desc: None,
        instruction_source: None,
        };

        assert_eq!(ctx.knot, knot);
        assert_eq!(ctx.loom_id, loom_id);
        assert_eq!(ctx.all_knots, all_knots);
        assert_eq!(ctx.rig_dir, rig_dir);
    }

    /// `BuildContext` can hold multiple knots from different looms.
    #[test]
    fn build_context_holds_multiple_knots() {
        let knot1 = KnotBuilder::new("plan-creator").build();
        let knot2 = KnotBuilder::new("plan-validator").build();
        let all_knots = vec![knot1.clone(), knot2.clone()];

        let ctx = BuildContext {
            knot: knot1.clone(),
            loom_id: LoomId("planning-loom".to_string()),
            all_knots: all_knots.clone(),
            rig_dir: PathBuf::from("/tmp/rig"),
            strand_queue: None,
        self_continuation_desc: None,
        instruction_source: None,
        };

        assert_eq!(ctx.all_knots.len(), 2);
        assert_eq!(ctx.all_knots[0].id, knot1.id);
        assert_eq!(ctx.all_knots[1].id, knot2.id);
    }

    /// A no-op provider implementing `ContextProvider` returns empty string.
    #[test]
    fn no_op_provider_returns_empty_string() {
        struct NoOpProvider;

        impl ContextProvider for NoOpProvider {
            fn build_context(&self, _input: &BuildContext) -> String {
                String::new()
            }
        }

        let provider = NoOpProvider;
        let ctx = BuildContext {
            knot: KnotBuilder::new("test").build(),
            loom_id: LoomId("test-loom".to_string()),
            all_knots: vec![],
            rig_dir: PathBuf::from("/tmp/rig"),
            strand_queue: None,
        self_continuation_desc: None,
        instruction_source: None,
        };

        let result = provider.build_context(&ctx);
        assert!(result.is_empty());
    }

    /// `ContextProvider` trait can be used polymorphically — a vector of
    /// providers can each build context and results can be concatenated.
    #[test]
    fn context_provider_trait_is_composable() {
        struct PrefixProvider(&'static str);

        impl ContextProvider for PrefixProvider {
            fn build_context(&self, _input: &BuildContext) -> String {
                self.0.to_string()
            }
        }

        let providers: Vec<Box<dyn ContextProvider>> = vec![
            Box::new(PrefixProvider("A\n")),
            Box::new(PrefixProvider("B\n")),
        ];

        let ctx = BuildContext {
            knot: KnotBuilder::new("test").build(),
            loom_id: LoomId("test-loom".to_string()),
            all_knots: vec![],
            rig_dir: PathBuf::from("/tmp/rig"),
            strand_queue: None,
        self_continuation_desc: None,
        instruction_source: None,
        };

        let combined: String = providers
            .iter()
            .map(|p| p.build_context(&ctx))
            .collect();

        assert_eq!(combined, "A\nB\n");
    }
}

