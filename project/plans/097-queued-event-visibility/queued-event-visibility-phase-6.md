# Phase 6 (gated increment): `EventCoalesced` hash-dedup backstop

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

> **GATE — do not start this phase unless the gate is met.** Enter **only if**
> post-release field evidence (the `queue-busy` lines + service logs from a live
> rig) still shows duplicate *invocations* on identical queued events. Record the
> gate decision (and the evidence) in Deviations before starting.

**Runs only after Phase 5 is green and committed, and the gate above is met.**

## Checklist
- [ ] Record the gate decision + the field evidence that triggered it (the
      `queue-busy=<n>` lines + service-log duplicates) in Deviations.
- [ ] Before the next invocation, drop a queued entry whose strand file is
      hash-identical to one already processed for that `(loom, knot, event-type)`
      in the current drain; log an `EventCoalesced` record with the dropped path
      and the retained path.
- [ ] Preserve per-item queue semantics (at-least-once, fault isolation) — this
      dedups *before* invocation, never inside a session.
- [ ] Add the `EventCoalesced` record to the `LoomEvent` set and its log-line
      render (`src/adapters/service_log.rs`); confirm the render test.
- [ ] Write the coalescing test suite — the phase's pane:
      - identical (hash-matching) entry dropped, distinct kept;
      - different event types never compared for coalescing;
      - at-least-once / per-item semantics unchanged (a dropped entry still
        leaves its retained twin to be delivered).
- [ ] Verification — `cargo clippy --all-targets` clean; the coalescing test
      suite green + `cargo test --test persistent_queue` still green (queue
      semantics); commit sha `⟨TBD⟩` recorded as this phase's external-verdict
      request. The ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record the gate decision + triggering field evidence, and any deviations -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- Advisory, not enforced — by design. (1)–(3) reduce duplicates; this phase is
  the enforced floor, and only for the identical case. The same-facts-but-
  different-text case remains the agent's judgment.
