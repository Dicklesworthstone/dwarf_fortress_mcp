# Current access to laboratory room furnishings

`Predicate::LaboratoryRoomAccess` proves structural access in the bounded
reference simulator. Its fixed policy is `dfmcp.laboratory-room-access/1`.
The predicate retains one explicit entrance, one bounded region and every
furnishing target; it is evaluated against the current snapshot whenever an
original goal, forecast or archived proof is checked.

## Meaning of access

The region contains at most 1,024 tiles on one level. The entrance and all
one to sixteen distinct furnishing targets must be inside it. Movement uses
cardinal neighbors whose observed canonical terrain is FLOOR. Buildings and
all furnishing target tiles block traversal, including planned furnishings
that have not yet been installed. Each target must have at least one adjacent
floor tile reachable from the original entrance.

This model treats every building footprint as an obstacle, including doors.
It describes structural floor access from the selected point. It does not
establish native DF pathfinding, actual hauling, unit access permissions,
connection of an arbitrary selected point to the whole fortress, bedroom
ownership, or a sleeping/needs benefit.

The predicate is true when every target has a currently established approach.
It is false when even allowing unknown terrain cannot provide every approach
inside this exact region. It is unknown when missing terrain could contain a
necessary route, or the complete qualified building census is unavailable.
A route outside the supplied region does not satisfy this bounded claim.

## Current occupancy and source authority

The complete building domain is inspected, including canonical aliases of the
building kind. Its bound is 65,536 entities. Qualified laboratory lifecycle and
full footprint facts establish occupied tiles. A malformed or unqualified
building record cannot silently become free space. Valid cancelled generic
buildings and cancelled/released furniture no longer obstruct the route;
marked furniture history must still establish the registered original
placement identity before its old footprint can be ignored.

The reference model's trusted laboratory evidence owner is required to use
this predicate as a proof. An arbitrary hash-valid snapshot, caller-provided
source label, replay fact or live scoped grant cannot acquire laboratory
authority. Raw queries remain structural inspection rather than admission.
Terrain absence stays unknown, even within the complete laboratory entity
universe.

Occupancy is rasterized with bounded rectangle differences, then two cardinal
searches distinguish established and potential routes. Both predicate-tree
shape and query work budgets include these extra census/search costs. All
shape checks run before allocation and recursive evaluation; duplicates,
out-of-region targets, multiple levels and oversized work refuse explicitly.

## Durable proof and concurrency

Canonical and plan-archive predicate tags append the new variant while
preserving all earlier tags and bytes. Target order is normalized by level,
row and column; duplicates remain invalid after normalization. Archived
reconstruction retains the exact entrance, domain and furnishing targets.

Read witnesses include the complete building domain as well as the terrain
region. Inserting a previously unseen building, moving a footprint, changing
a lifecycle or removing a building invalidates the old occupancy witness.
Existing witness certificates that do not use this predicate retain their
original representation. Compilers that also select inventory, retain installation custody or choose
routes must recheck those additional inputs at a changed anchor.

A later obstruction can change current goal truth without rewriting an old
receipt or the first-achieved historical record. A compile-time path or a
collection of floor predicates alone cannot establish this current access
claim.
