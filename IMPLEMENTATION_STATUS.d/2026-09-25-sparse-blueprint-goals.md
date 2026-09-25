### Sparse multi-level terrain goal evaluator

Added `scripts/excavation_blueprint.py`: bounded disjoint cuboids and exact shape
masks, coherent map/1.5 evaluation, sampled whole-mask stability and exact bounded
remaining-cell diagnostics. See `docs/EXCAVATION_BLUEPRINTS.md`.

Twelve actual Python tests pass, including 4,608 shape/liquid/designation cases
and 500 legacy-floor differential traces. The unchanged decoder/client source
used for execution matches Git blob `95a8f485afa76baced77edac0702ddb891b945cb`.
This increment is a pure library; durable blueprint CLI integration remains next.
No Rust, native SDK, live-game, power-loss, whole-repository or admission evidence
is claimed. Owner: `df-action-coordinator-exec-ero.4`; the broad bead remains open.
