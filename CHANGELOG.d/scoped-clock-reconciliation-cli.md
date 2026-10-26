## Execute bounded recovery directly from discovered clock history

- Add recover_scoped_clocks.py using independently retained complete inventory
  identity, run-only query credentials and existing read-only original Batch.
- Return complete original state, pending counts and retained per-record outcomes;
  preserve partial results on failed queries with exit two and no retry.
- Keep both owners through one stdout attempt; refuse stale selection, invalid
  arguments, missing authority or oversized output before native contact.
- 58 tests and eight assertion-rejected mutants pass on unchanged measured inputs.
  Real clock TCP/private files and complete CLI execute; original batch/budget
  are explicit ports. No native, stored-format or production-admission changes.
