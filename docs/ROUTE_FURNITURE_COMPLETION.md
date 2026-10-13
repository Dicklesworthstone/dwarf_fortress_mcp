# Complete original route-batch construction goals

Beads: `df-dfhack-bridge-plane-c-pic.4/.5`; WP-10. Route placement receipts
prove historical construction-job registration, not completed furnishings.
This increment joins the complete original batch to the existing receipt-linked
whole-plan construction condition instead of accepting an arbitrary successful
subset of receipts as the original objective.

`route_furniture_completion.Goal` retains the complete exact-item plan and its
DAG, original route review digest, original batch path/identity, original build
manifest, original operations incarnation, all original Placed receipts and one
fixed construction deadline/cadence/stability policy. Its bounded `DFMRGC01`
encoding is additive; no native wire, placement journal or old monitor changes.
Decoding requires exact re-encoding. The goal digest covers the complete object.
A digest identifies bytes, not authorization, acquisition or source custody.

`from_batch` requires a verified, locked Batch with all original steps placed
and registered. Receipt keys, kinds, items, targets, incarnation and chronology
must agree with that complete original plan. Missing, uncertain, cancelled,
foreign or unregistered children refuse. `verify_origin` rechecks the parent,
complete original plan and exact receipts, including for terminal history.
The original route contract stays in the unchanged parent review; the monitor
retains its identity and never treats construction progress as current routing.

Every sample uses the unchanged complete-roster decoder and whole-plan condition.
The first sample must already match both original native incarnations/software;
a new source is not learned from convenient first-sample data. Every selected
building and installed item is assessed in the same capture. Completion requires
all conditions together for the original advancing-tick stability window; prior
individual successes are not latched. Missing/replaced buildings, active removal,
unverified installed items, source regression, deadline or sample exhaustion
cannot become success. An interrupted read resets unfinished stability.

Core evidence: 16 actual Python tests pass, including every prefix truncation of
a fixture, all 32 original targets, changed selections, source mismatch, complete
plan binding, fixed timing, cancellation callbacks and real operations decoding.
The parent Batch API is injected in these core tests; this is not private-parent,
transport, native SDK, live game, Rust/MCP or full-workspace qualification.
The four reused modules were reconstructed and Git-blob checked without changes.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -W error::ResourceWarning \
  -m unittest test_route_furniture_completion -v
```

Durable monitoring and executable acquisition are separate integration work.
No placement permission, retry, effect discharge or production admission is added.
