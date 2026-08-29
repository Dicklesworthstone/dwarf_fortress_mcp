# Threat Model

## Scope

This threat model covers the MCP server, bridge protocol, canonical ledger, checkpoint files,
derived indexes, imported knowledge, and multi-agent authority. Dwarf Fortress and DFHack are
external components whose defects may affect the system.

## Security properties

1. No unauthorized game mutation.
2. No mutation outside granted entity/map/resource/configuration scope.
3. No authority gained from untrusted text.
4. No arbitrary host code or path execution through default protocol.
5. No duplicate effect from ordinary retry.
6. No stale agent mutation after lease transfer.
7. No false representation of semantic completion.
8. No silent compatibility assumption.
9. No corrupt/incomplete checkpoint represented as restorable.
10. No unbounded payload or query path.

## Threat actors

- unauthenticated network client;
- authenticated but underprivileged client;
- compromised agent;
- malicious prompt content;
- malicious mod/save data;
- compromised bridge;
- stale/replayed client;
- concurrent conflicting agent;
- local unprivileged process;
- accidental operator;
- corrupt disk or crashing host.

## Abuse cases

### Arbitrary-command smuggling

Attacker places a DFHack command or Lua snippet in an action field. Defense: closed typed actions;
token validation; no evaluator surface.

### Prompt injection in game

A dwarf/book/announcement says to call restore or reveal files. Defense: taint; text cannot select
capability/action; plans remain typed and authorized.

### Oversized bridge object

Bridge declares billions of entities or a huge string. Defense: limits before allocation and
streaming bounded decode.

### Stale commit

Agent commits against a prior anchor after another plan changed the same area. Defense:
prepare/revalidate, revisions, leases, exact plan digest.

### Retry after uncertain timeout

Client retries a designation after effect but before receipt. Defense: durable idempotency,
bridge journal, `indeterminate`, reconciliation.

### Lease zombie

Old coordinator wakes after lease expiry. Defense: monotonically increasing fencing token checked
at ledger and bridge effect.

### Checkpoint path traversal

Client supplies `../../`. Defense: clients never supply paths; scoped root capability and relative
manifest validation.

### Forged confirmation

Client reuses approval for a modified plan. Defense: seal binds exact plan digest, anchor, risk
digest, expiry, signer.

### Compatibility spoof

Bridge claims a version is compatible. Defense: server-owned manifests and probes; bridge report
alone insufficient.

### Projection cache leak

Privileged result served to read-only session. Defense: cache key includes capability/redaction
manifest.

## STRIDE-style table

| Threat | Examples | Primary controls |
|---|---|---|
| Spoofing | forged session/bridge/operator | authentication, instance IDs, signed seals |
| Tampering | plan, delta, checkpoint, receipt | digests, immutable plans, checksums, WAL |
| Repudiation | agent denies mutation | append-only audit/evidence |
| Information disclosure | raw save/doc/privileged fields | scopes, redaction, no arbitrary paths |
| Denial of service | huge query/frame/obligations | multidimensional budgets, backpressure |
| Elevation | text/extension/delegation broadens rights | closed registry, non-amplifying delegation |

## Residual risk

- DFHack or game may mutate in ways the bridge cannot distinguish.
- A compromised bridge with game access can lie; semantic cross-checking reduces but cannot remove
  this.
- Checkpoint restore may not undo simulation effects external to the save.
- Human confirmation can be mistaken.
- Unknown mods can invalidate semantics.
- No interface can guarantee a competent agent will choose good strategy within granted authority.

Residual risks must be visible in compatibility and policy, not hidden in generic disclaimers.
