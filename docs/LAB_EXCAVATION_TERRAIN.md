# Executable laboratory excavation terrain

The reference excavation engine uses a closed source-to-target terrain rule.
A designation cannot create a floor, stair or ramp out of open space or a
chasm, and cannot erase trees, magma walls or unsupported tile codes. The same
rule applies to fresh dispatch, retained active designations and every physical
tile replacement during an advancement interval.

Owners: `df-lab-connected-room-access-7c2` and
`df-predicate-evidence-authority-r7m`. This is the deterministic laboratory's
coarse terrain model. It does not qualify a native DFHack operation or establish
native excavation safety, material origin, directional stairs or cave-in rules.

## Supported transformations

| Designation | Target | Admitted source when work is required |
| --- | --- | --- |
| Mine | FLOOR | SOLID_WALL, FORTIFICATION, STAIR, RAMP |
| RemoveConstruction | FLOOR | SOLID_WALL, FORTIFICATION, STAIR, RAMP |
| Channel | OPEN_SPACE | SOLID_WALL, FLOOR, FORTIFICATION, STAIR, RAMP |
| UpStair | STAIR | SOLID_WALL |
| DownStair | STAIR | SOLID_WALL, FLOOR |
| UpDownStair | STAIR | SOLID_WALL |
| Ramp | RAMP | SOLID_WALL |

A tile already equal to the registered target is a valid no-op. It consumes no
excavation service. This lets an interrupted room reuse floor already excavated
by another valid action, or a channel accept already-open cells. CHASM remains
distinct from OPEN_SPACE; it is not silently normalized into a completed channel.
Missing, malformed and explicitly UNKNOWN terrain remain unavailable.

The existing single STAIR code represents generic laboratory stairs. The three
stair modes retain that target without claiming native direction or connectivity.
RemoveConstruction retains its historical abstract reference transformation;
it neither proves a native construction's provenance nor returns materials.

## Dispatch and progress

`effects::validate_dig_terrain` checks the bounded exact cuboid before creation
of a designation. It rejects the whole request if any selected cell cannot make
the requested transition. No tracking entity or partial designation is created
on that refusal.

At the beginning of an advance, retained mode/target identity and current terrain
are validated before any physical progress. A source that became void or a
hazard since dispatch therefore cannot finish mining or consume unrelated work
time. A tile already at its target remains eligible even if another actor
completed it while the designation's old remaining counter was retained.

The engine also rechecks a cell inside the existing bounded chunk decode,
immediately before replacement. This preserves the original per-tile work
allowance, including large excavation batches. Two overlapping
designations can both be admissible at an ordinary wall, yet conflict after one
channels it into open space during the same interval. The second operation must
not recreate ground. The advance returns a precondition refusal and the owning
transaction discards its shadow, so no partial event becomes canonical. Explicit
cancellation can retire the conflicting designation before a later reviewed
advance. Cancellation itself does not require the old terrain to remain diggable.

This is a physical transition check, independent of worker selection, materials,
original action identity, observation cadence and proof. Those existing checks
still apply. A successful designation is still pending work; only later eligible
observations can prove its sealed postconditions and stability requirement.

## Blueprint planning

Room, furnished-room and equipped-workshop blueprint compilation checks each
selected excavation cell against this same rule after the existing complete
hazard-halo checks. A fully observed void is not a viable room site merely
because it contains no magma. The error identifies the selected coordinate,
source terrain and designation.

Only actual excavation parts are subject to the transformation rule. A void,
tree or chasm outside the requested parts does not become a new excavation
request. A defensive moat retains its exact perimeter, leaves its interior
untouched and can reuse already-open channel cells. Existing magma-halo and
unsupported-span restrictions remain in force. Repeated halo scans and selected
cell visits share the existing bounded preflight work allowance.

Direct action requests remain inspectable proposals. Their discarded-fork
forecast reaches the same dispatch and progress validation as actual execution;
an unavailable terrain transition is a refusal, not a predicted successful room.
No action, predicate, prepared-plan or archive encoding changes. Historical
receipts are not rewritten, and a later execution or forecast does not inherit
permission to repeat a formerly overpermissive terrain transformation.
