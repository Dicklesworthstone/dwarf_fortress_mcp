## Recover retained uncertain furnishing placements

Original furnishing batches can now query and durably reconcile an Indeterminate
receipt using read authority, without placement permission or redispatch. Offline
resolved-receipt recovery and uncertain cancellation retain their previous scope.
Final query authority is checked for all profiles. Exact-item recovery imports
are independent of the unused room-planning graph; explicit handoff profiles
still require their existing validators. Sixteen real-file/TCP/CLI component
tests pass with warnings denied. See `docs/FURNITURE_BATCH_RECONCILIATION.md`.
