## Original furnishing prefix execution

Beads: df-dfhack-bridge-plane-c-pic.4/.5; broad execution remains open.

Implemented a sealed 1..8-step original furnishing prefix using the actual
existing FurniturePlan/progress model. The complete checked original inventory,
exact keys/items/targets/dependencies, and room/terrain metadata bind the seal.
Only one next original child can change at each transition; prior receipts and
all future selections remain immutable. No retry or completion authority exists.

The executable `run_furniture_batch.py review|run` rebuilds confirmation from
the original Batch, then delegates fresh per-step native review/advance to the
existing owner. No native client, setter, journal framing or recovery behavior
is replaced. One shared wall/work/RPC/byte budget spans every call and original
terrain reopen; full output is reserved before contact. Original source and
current operator remain fixed. Existing per-step locks/CAS protect concurrent
changes; final original batch/terrain custody stays held through the sole output
write. Unknown/refused/cancelled work never permits the next step.

Executed 50 Python tests with warnings denied:
- 20 pure-policy cases, including 3,384 count/offset selections across 1..32 steps;
- 24 new executor/CLI cases using deterministic original-owner/native doubles;
- six actual existing public-function boundary cases with lower-level doubles.

Coverage includes eight steps within a complete 32-item plan, new confirmation
for later original work, lost second reply/retained uncertainty, concurrent
inventory changes, stop, source/authority/terrain loss, fixed shrinking budgets,
pre-contact output admission and final output refusal without repeat dispatch.

The six boundary cases execute unchanged fetched `furniture_batch.py` function
bodies (`review`, `advance`, `select_next`, `review_seal`) selected by AST. In the
partial local checkout their source excerpts were supplied using the test-only
FURNITURE_PREFIX_TEST_SOURCE override; repository tests default to the actual
source file. Owner storage, captures and low-level placement are test doubles.
The production command exposes no backend or source selector.

The real unchanged furniture_plan.py dependency was verified against Git blob
52fe43ce2d6514f3f0d1a3d983cc53c31fb39143. New runtime/test uploaded blob hashes
match the tested local files. Python syntax checks pass. Existing owner/native
modules are not edited. The full native/import graph, real journal recovery,
TCP/protobuf receipt integration, DFHack producer, Rust workspace, live-game,
full-repository qualification and production admission were NOT executed or
claimed. These are component/API-boundary results, not native effect evidence.
