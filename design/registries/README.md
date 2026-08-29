# Frozen design registries

This directory turns the architectural prose into reviewable, diffable control tables. The
comprehensive plan is the normative design narrative; these registries are its compact execution
index.

A registry entry may be in one of four states:

- **frozen** — the identifier and meaning are stable for `dfmcp/0`; incompatible changes require a
  new protocol or schema version;
- **provisional** — implementation may refine fields without changing the semantic promise;
- **experimental** — opt-in and not part of the compatibility floor;
- **retired** — retained only for decoding and migration.

Identifiers are never reused. Deleting an entry means marking it retired, not renumbering the
registry. Every production work package must link to the invariants, effects, errors, schemas, and
tests it changes.

| Registry | Purpose |
|---|---|
| [`INVARIANTS.md`](INVARIANTS.md) | Fifty non-negotiable correctness and safety properties. |
| [`CAPABILITIES.md`](CAPABILITIES.md) | Authority vocabulary, scope rules, and delegation limits. |
| [`ACTIONS.md`](ACTIONS.md) | Semantic mutation and control vocabulary. |
| [`EFFECTS.md`](EFFECTS.md) | Effect boundaries and required handling disciplines. |
| [`ERRORS.md`](ERRORS.md) | Stable error codes and retry/reconciliation semantics. |
| [`DETERMINISM.md`](DETERMINISM.md) | Inputs that must be captured for replay equality. |
| [`SCHEMAS.md`](SCHEMAS.md) | Versioned wire, ledger, and evidence schemas. |
| [`TESTS.md`](TESTS.md) | Test families and minimum negative evidence. |
| [`WORK_PACKAGES.md`](WORK_PACKAGES.md) | Delivery graph and acceptance dependencies. |
| [`COMPATIBILITY.md`](COMPATIBILITY.md) | DF/DFHack/MCP compatibility policy. |
