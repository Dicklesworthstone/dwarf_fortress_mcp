# Rust furniture journal positive-proof recovery

The source now includes BuildJournal::reconcile_receipt and an exact append-only
Indeterminate-to-Placed transition. It uses original-key Query authority, exact
source checks and existing synchronized custody; it cannot dispatch or cancel.
Ordinary receipt successors remain strict. The original uncertain frame and
both dispatch/cancellation history flags remain retained through reconciliation.

Eight actual-journal Rust regression tests are added, but rustc/cargo/rustfmt are
unavailable in the editing environment and none have been executed there.
Source/upload byte identity checks are not Rust or native/live qualification.
This first increment is an adapter API; session/MCP routing remains separate.
See docs/RUST_FURNITURE_RECONCILIATION.md. Beads df-dfhack-bridge-plane-c-pic.4/.5
remain open for their broader integration and qualification obligations.
