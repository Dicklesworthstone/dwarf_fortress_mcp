# Witnessed work-detail membership control

This development slice addresses the missing labor-configuration edge between
workforce planning and actual workers. It changes membership in one existing
**OnlySelectedDoesThis** work detail for 1..32 explicitly selected citizens.
It does not create work details, change their modes/allowed labors, select raw
labor enum numbers, assign jobs, interrupt jobs, or promise future productivity.
The separate native profile is reserved as **workforce/1.17**.

## Semantics

The paused precondition capture contains exact world folder/site, bridge
generation/dispatch sequence/tick, automatic-profession enablement, the complete
bounded work-detail list, all assignment lists and allowed-labor masks, and each
selected citizen's native ID, historical-figure ID, eligibility and labor mask.
Labor keys are carried in native array order, not guessed from enum numbers.
A detail index is meaningful only with this complete capture, not as a persistent
entity ID. Name, mode/flags, any detail membership, citizen identity, eligibility,
or labor changes invalidate preparation. This does not fence external UI/plugins.

Plans bind a detail index, desired membership and the exact capture SHA-256.
Keys are immutable. No-op membership requests, non-selected-only/empty details,
unpaused sources, disabled automatic professions, ineligible selected citizens,
over-capacity lists and stale witnesses fail before effect dispatch. Preparation
lasts at most 60 monotonic seconds and replay does not renew its lifetime.

The engine computes the complete replacement membership and changed-citizen list
before the mutation boundary. It records Unknown and advances its dispatch fence
before the writer changes anything. The native integration must stage its vector
and resolve all pointers before swapping membership, then call DFHack's supported
`Units::setAutomaticProfessions` only for changed citizens, like its manipulator.

Readback verifies exact membership and all other captured configuration. Only the
labor masks of changed citizens may differ. Adding membership must actually enable
every allowed labor in that detail. Removing membership does **not** prove those
labors are disabled: other details may also grant them. Every selected post-mask
and the complete post-capture witness are retained. This is immediate configuration
evidence, not exclusive assignment, current state at a later query, or job success.

An exception or incorrect readback after the boundary retains Unknown and blocks
new keys. Commit never retries Unknown/Applied/Refused/Cancelled operations.
Cancel can retire a Prepared record but cannot undo an uncertain or applied write.
There is no automatic rollback. World/map reset invalidates local tokens; a missing
record afterwards is unknown history, not proof of non-application. Native records
are bounded to 64 and never evicted during an incarnation.

Bounds: 32 selected units, 64 details, 4,096 aggregate memberships, 128 labor
columns, 64 KiB/capture, 8 KiB/effect. Source tests use injected callbacks; values
never retain native pointers. Existing native protocols and production admission
are unchanged. No Rust/MCP or live qualification is implied.

## API basis

DFHack's `gui/manipulator.lua` toggles sorted `work_detail.assigned_units` and calls
`dfhack.units.setAutomaticProfessions`. The Units API describes recomputation from
work details; the autolabor overlay warns that disabled automatic professions
make work-detail edits ineffective. This slice refuses that disabled state.
Reviewed upstream sources (not an SDK qualification):

- DFHack/dfhack revision `1f2a33feffd4c30ffeb193be6dea832ea06f589f`,
  `library/include/modules/Units.h`, `plugins/lua/autolabor.lua`.
- DFHack/scripts revision `0d0b2f5fdf518ffaf5ff04a12864cda57056beaf`,
  `gui/manipulator.lua`.
- DFHack/df-structures revision `3bfa5aa5ae3fdbe4e77d22f8125b1823c5847ccc`,
  `df.plotinfo.xml` (`work_detail`) and `df.game_g.xml` (external flag).

## Executed engine evidence

`python3 scripts/test_workforce_native.py --mutations` executes the real header.
GCC and Clang each passed 8,497 assertions with warning denial and nonrecovering
UBSan. Three independently encoded capture/record vectors agree. Three separately
compiled mutants per compiler are rejected: bypassed witness validation, bypassed
readback validation, and permitted retry after an unknown effect. The membership
matrix covers all 256 eight-citizen subsets for both requested membership values.

These are executable callback-double tests, not an SDK build, live fortress,
filesystem durability, Rust/MCP, full qualification, or admission. Native RPC and
durable operator integration are separate increments. No bead is closed here.
