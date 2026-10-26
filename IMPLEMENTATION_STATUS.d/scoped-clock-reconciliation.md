# Scoped-clock reconciliation and executable recovery

Implemented bounded query-only recovery of discovered original next-step and
final-construction clock records through recover_scoped_clocks.py. Independent
complete inventory identity, exact native intents, isolated run-only authority,
same-budget I/O and private history checks gate queries and terminal publication.
No effect retry or new-effect admission is possible through this owner. Original
files are never repaired. Complete original state and partial retained outcomes
survive ordinary query failure; custody/authority loss still refuses publication.

58 actual Python tests pass: 27 unchanged inventory, 19 recovery and 12 CLI cases.
Eight weakened implementations fail assertions. All 15 measured source/test/runner
inputs remain unchanged. Actual clock TCP/codec/private-file and complete CLI,
main/runpy and output paths execute against joined synthetic peers; original
route-batch custody and budget are explicit ports. No real private route-batch,
DFHack SDK/game, Rust/MCP, full workspace or production-admission qualification.
See docs/SCOPED_CLOCK_RECONCILIATION.md. Broader beads remain open.
