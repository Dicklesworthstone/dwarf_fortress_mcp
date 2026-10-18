## Execute confirmed original furnishing prefixes

Add `run_furniture_batch.py review|run` for the next exact 1..8 original
furnishings, preserving the complete 32-step plan, fixed items/targets, dependency
order, original keys and room/terrain custody. A bounded confirmation policy
rejects skipped, repeated or substituted children and retains prior receipts.

The executable runner delegates to the existing single-step review/advance
owner with one shared wall/work/RPC/byte allowance and pre-contact output
reservation. It stops at blockers, uncertain/refused work, changed source,
credentials, custody or budget. Final original custody is held through output;
no automatic retry, recovery, unpause, native method or journal format is added.

Fifty Python tests pass: 20 policy, 24 owner-double executor/CLI, and six actual
existing public-function boundary cases with lower-level doubles. The policy
matrix covers 3,384 original-prefix selections. This is component/API-seam
validation, not native receipts/private-file execution or live qualification.
See `docs/FURNITURE_PREFIX_RUN.md` for confirmation, recovery and evidence scope.
