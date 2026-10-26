## Preserve selected-record recovery alongside inventory reconciliation

- Keep the existing recover_route_furniture_clock.py inspect/query/cancel tests,
  runner and contract at their exact pre-change blob identities. The multi-record
  reconciliation suite now uses distinct test/check/contract filenames.
- Update new CLI fixture imports and documented verification command. No native
  implementation, retained format, authority or admission behavior changes.
- Rerun all 58 new/inventory tests and eight assertion mutants after separation;
  unchanged hashes qualify the final new suite only, not an unrun older suite.
