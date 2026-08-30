# Operations

## Startup

1. load safe defaults and policy;
2. open/recover ledger;
3. verify schema/migrations;
4. scan incomplete checkpoints;
5. start bridge supervisor;
6. negotiate bridge/game manifests;
7. reconcile unresolved actions;
8. ingest full snapshot if epoch continuity is absent;
9. rebuild/validate derived indexes;
10. run startup doctor;
11. admit read-only sessions;
12. enable mutation families whose compatibility gates pass.
13. start the agent-session transport when expected: `dwarf-fortress-mcp serve` runs the pinned
    fastmcp_rust stdio server (MCP 2026-07-28, modern-only; ADR-013);

Critical uncertainty starts safe mode.

## Safe mode

Safe mode permits bounded observation and doctor according to policy. It disables:

- new mutation;
- clock changes;
- automatic retry of unresolved actions;
- restore/repair apply without explicit seal;
- unverified compatibility families.

## Graceful shutdown

1. stop admitting mutation;
2. request session drain;
3. prevent new step dispatch;
4. resolve in-flight bridge requests;
5. persist receipts;
6. cancel/drain obligations within budget;
7. mark remaining effects indeterminate if necessary;
8. release or expire leases;
9. checkpoint ledger;
10. stop bridge/index workers;
11. emit shutdown report.

A timeout does not erase active work.

## Bridge reconnect

- compare bridge and game instance IDs;
- query idempotency journal;
- re-handshake compatibility;
- reconcile unresolved operations;
- full snapshot/new epoch when continuity cannot be proven;
- re-enable action families granularly.

## Ledger recovery

Doctor classifies:

```text
prepared_no_dispatch
dispatch_no_receipt
receipt_no_observation
observation_no_terminal_transition
active_obligation
stale_lease
incomplete_checkpoint
corrupt_frame
migration_incomplete
```

Each class has a deterministic safe next step.

## Checkpoint operations

Checkpoints should be labeled with plan/reason, source anchor, versions, and manifest digest.
Staging directories are quarantined after failure and never advertised as complete.

## Restore

Restore is maintenance mode. It drains all sessions, uses a global fence, verifies exact seal,
materializes staged replacement, reloads, creates a new epoch, and invalidates stale plans.

## Upgrades

- pre-upgrade doctor bundle;
- stop mutation;
- drain and backup;
- transactional migration;
- re-handshake/probe;
- post-upgrade full snapshot if needed;
- post-upgrade doctor;
- capability-family re-enable.

## On-call triage order

1. protect save and stop new effects;
2. determine whether any effect is indeterminate;
3. preserve bridge journal and ledger;
4. capture doctor bundle;
5. identify earliest anchor/receipt divergence;
6. reconcile before retry;
7. apply only sealed repairs;
8. retain artifacts and update negative-evidence ledger.

## Logging

Logs use stable event codes and IDs. Never log full saves, secrets, arbitrary in-game text, or
unbounded payloads by default. Evidence stores digests and bounded excerpts.

## Capacity

Operators set hard ceilings for sessions, observations, obligations, ledger bytes, checkpoint
bytes, and derived indexes. Near limits, the system backpressures and preserves mutation evidence
before lower-priority indexing.
