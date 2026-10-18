# Original furnishing prefix

Beads: `df-dfhack-bridge-plane-c-pic.4/.5` (WP-05/WP-10).

`furniture_prefix.Prefix` seals the next exact 1..8 steps of an existing complete
furnishing batch. It preserves the actual furniture plan's canonical dependency
order, fixed items, coordinates, original keys and complete inventory. The seal
also binds original room/terrain metadata when present; these are not replaced
by a smaller selected plan. Prefix counts are exact, never silently clamped.

This first increment is pure policy. The ordinary batch owner must authenticate
all journals, receipts and source/custody facts before passing its inventory.
An imported inventory or self-consistent seal grants no effect authority.
Only one newly recorded original child can advance the prefix. Earlier and later
children must remain unchanged. Unknown, indeterminate, refused or cancelled
outcomes stop progress; they cannot authorize the next step or a replacement key.

Twenty executed Python tests use the real unchanged `furniture_plan` model and
synthetic inventory inputs. The exhaustive count/offset matrix covers 3,384
selections within 1..32-step plans, including eight-step suffixes, dependency
ordering, altered history, cancellation, exact types and original terrain scope.
No native, filesystem, CLI, Rust or live execution is established by these tests.
