# Reviewed civilian release in the reference laboratory

The stable-clear FSM now has a complete reference-world census and an ordinary
guarded release compiler. These are separate responsibilities: the process-owned
monitor establishes a clear interval, and the release predicates establish
current safety before every effect. Neither a caller-supplied history nor a
matching snapshot hash grants clearance authority.

The original `civilian_protection_v1` request remains the named protection
workflow. Its pause policy and canonical bytes are unchanged. This document
describes the additive `dfmcp.laboratory-civilian-release/1` model, not native
Dwarf Fortress threat detection, evacuation, burrow suitability or pathfinding.

## Qualified complete census

`laboratory_civilian_census(snapshot, selected_burrow)` is for an observing owner
that already owns the complete reference snapshot. It inspects the complete
bounded entity domain, rather than interpreting a selected query page as the
population. The entity-map keys must match their record identities. Canonical
kind names, including reserved-name aliases, are counted consistently so an
alias cannot hide a unit or creature from the census.

The classifier consumes only facts admitted by `laboratory_fact_value`: known,
internally consistent presence, observation time no later than the snapshot,
and the registered `dfmcp.lab-scenario/1` or `dfmcp.reference-effects/1`
derivation with its existing zero-digest convention. Assertions, replay,
unregistered derivations, native field labels, changed producer digests, future
facts and unavailable presence do not establish a role or clear threat.

The closed reference interpretation is:

| Domain | Required facts and interpretation |
| --- | --- |
| Living civilians | A nonzero Unit identity and generation, qualified `alive=true`, and a civilian role. |
| Civilian role | An omitted `squad` field retains the existing reference model's nonmilitary default. Qualified `squad=Null` is also civilian. A present unavailable or malformed role makes the roster incomplete. |
| Military role | A qualified `squad=Entity(id)` must name a current Squad with a nonzero identity and generation. A dangling assignment makes the civilian roster incomplete. |
| Dead units | Qualified `alive=false` excludes the unit from the current living roster. An unknown life state never excludes it as if known dead. |
| Benign creatures | Qualified `hostile=false` is nonhostile in this model. |
| Hostile creatures | Qualified `hostile=true` requires a qualified closed `threat_status`: `approaching` and `attacking` are Present; `slain` is clear for that creature. Other, missing or unavailable statuses are Unknown. |

Any positively observed active threat makes the assessment Present. Otherwise,
any incomplete creature fact makes it Unknown. Clear requires the entire current
creature domain to be classified. Absence is usable only through the trusted
complete reference-world contract; imported or projected snapshots cannot acquire
that contract by carrying familiar strings.

The census reports exact civilian IDs and generations, the selected burrow
incarnation, current threat assessment, roster completeness, configuration truth,
an explicit inspected-entity count, and a stable source-scope digest. The digest
binds the policy, fortress, selected burrow, relevant entity incarnations and
normalized life/role/threat domain. Memberships, standing orders, revisions,
cursor, elapsed ticks and ordinary source refreshes between the two admitted
reference derivations do not change that scope. Roster, role, generation,
threat-domain or source-eligibility changes do.

## Observed protection and release

`protected=True` means that some protection is positively retained: either the
selected burrow has a qualified living civilian membership or the global
restriction is ACTIVE. Partial assignments are therefore conservative retained
protection. This value does not prove every civilian is assigned or has arrived.

`released=True` requires a complete civilian roster, explicit false membership
for every civilian in the selected burrow, the current selected burrow identity,
and an explicitly INACTIVE restriction at the deterministic fortress settings
entity. Missing legacy membership bits and missing settings are Unknown. They
cannot be used to confirm peaceful release. The standing-order identity helper
is shared by the world classifier and reference effects; its existing bytes and
namespace are preserved.

## Per-effect safety and exact generations

`Predicate::LaboratoryCivilianReleaseSafety { burrow, civilians }` names exact
`LaboratoryEntityIdentity { id, generation }` values. It evaluates True only when
the selected burrow matches, the entire current qualified civilian roster equals
the sealed roster, and the complete current threat domain is Clear. Known danger
or a known changed roster/incarnation yields False; incomplete facts remain
Unknown. Neither False nor Unknown permits a release effect.

The predicate intentionally excludes membership and standing-order values, since
the release itself changes them. It also cannot establish a historical clear
window. The observing owner's retained decision must admit the reviewed source;
the ordinary planner and adapter then recheck this current predicate. Native
`PredicateEvidence::scoped` and untrusted evidence return Unknown for this
laboratory-only predicate, even with broad domain grants. No native capability
or compatibility admission is added.

The new canonical and exact-plan archive tag is **14**, appended after the
paired-stair policy tag 13. IDs use the existing big-endian u64 encoding and
generations use u32. Canonical predicates use the existing u64 collection count;
the exact archive uses its existing u32 count. All prior predicate tags and
payloads remain unchanged. Query continuation identity also binds the complete
roster and generations.

## Ordinary guarded release execution

`CivilianReleasePlan::compile` takes one exact burrow and 1..64 exact civilians.
It canonicalizes ordering while refusing duplicate IDs, zero IDs, zero
generations and a civilian ID equal to the burrow. It emits two existing actions:

1. Remove membership in the selected burrow for the complete sealed civilian
   roster, with current safety as a precondition and all removals as the proof.
2. Set `CIVILIAN_BURROW_RESTRICTION=INACTIVE` only after the membership step is
   verified, again checking current safety immediately before dispatch.

`requested_actions_for(fortress)` and `prepare` bind the final step to the complete
current goal: exact safe roster, all removed memberships and the inactive global
order. A historical Verified action remains historical evidence when a new
threat, civilian or generation later invalidates the current goal.

The release compiler accepts no caller-asserted clear history. Its embedding
owner must supply the independently retained, reviewed clear-window admission.
The compiler's `prepare` still requires an exact current anchor, fortress-wide
Observe authority, Plan authority, whole-census entity budget, action/byte
budgets, and the normal guarded checkpoint/effect policy. It does not require
pause: a running reference world can accumulate the required game-time window,
and each effect has its own current safety gate.

When a threat returns after membership removal, the dependent standing-order
change is refused. A new protection plan can restore the current roster. The
system does not rewrite the earlier removal's historical receipt or infer that
an inactive order is justified by that receipt.

## Cancellation keeps protection without crossing incarnations

The membership action explicitly compensates by assigning true; the restriction
action compensates by setting ACTIVE. Current cancellation and compensation
authority remain necessary. Cancelling an undispatched step never runs its
compensation.

For steps carrying the new release predicate, the reference adapter separately
checks the sealed civilian and burrow generations before applying a protective
compensation. Returned threats do not block protection. Missing/replaced units
or a replaced burrow do block it, so cancellation cannot configure a new entity
that reused an old ID. The operator can narrow cancellation to stop future work
without compensation and request fresh protection for the current roster. This
additive guard does not change legacy protection cancellation behavior.

## Bounds and executable evidence

The census admits at most 65,536 entity records and retains at most 64 civilian
identities; an oversized civilian roster is explicitly incomplete. Predicate
shape validation bounds aggregate census work before Boolean short circuits,
and query execution accounts for the full census cost for every candidate row.
The compiler emits exactly two actions and bounds duplicated encoding accounting
to 256 KiB. The process owner separately bounds retained per-burrow monitors.

Focused tests cover qualification failures, missing and military roles, complete
threat inspection, generation changes, partial configuration, stable source
scope during ordinary progress, native/untrusted refusal, canonical tag and
archive preservation, bounded query continuation identity, exact complete-goal
proof, real MemoryAdapter prepare/commit/poll/replay, changed safety between
dependent effects, revoked effect authority, protective cancellation after a
returned threat, and cancellation across reused IDs. These are reference and
component execution results; they do not claim native game qualification.
