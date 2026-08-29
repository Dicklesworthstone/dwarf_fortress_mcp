# Semantic action registry

**Status:** phase-zero core entries are frozen; DFHack encodings are provisional.

The registry defines meaning independently of a particular DFHack command, UI key sequence, or
memory layout. Each action family has a minimum risk, write scope, deterministic normalization,
required capabilities, postcondition template, and completion mode.

| Action | Minimum risk | Capability | Write scope | Completion | Required semantic proof |
|---|---:|---|---|---|---|
| `pause.set` | reversible | `control_clock` | fortress clock | immediate | normalized paused state equals target |
| `designation.dig` | guarded | `designate` | target cuboid/tiles | temporal | designation exists; optional excavation obligation reaches terminal condition |
| `building.construct` | guarded | `construct` | footprint + access halo | temporal | building exists at normalized footprint and construction is complete |
| `labor.set` | reversible | `configure_labor` | citizen × labor | immediate/eventual | normalized assignment equals target and no protected-role constraint failed |
| `work_order.create` | reversible | `configure_production` | order/workshop/product | temporal | normalized order exists; optional production count obligation completes |
| `stockpile.configure` | reversible | `configure_logistics` | stockpile/filter namespace | immediate/eventual | normalized filter and links equal requested state |
| `squad.assign` | guarded | `configure_military` | squad/member/position | immediate/eventual | normalized assignment equals target |
| `burrow.membership.set` | reversible | `configure_logistics` | burrow/entity set | immediate | membership relation equals target |
| `standing_order.set` | guarded | `configure_logistics` | allowlisted policy key | immediate | normalized policy value equals target |
| `checkpoint.create` | guarded | `checkpoint` | save slot | immediate/durable | manifest, content digest, and durability proof recorded |
| `checkpoint.restore` | guarded | `restore` | fortress epoch | temporal | new epoch full snapshot matches checkpoint identity |
| `extension.invoke` | registry-defined | registry-defined | registry-defined | registry-defined | extension-specific verifier |

## Normalization rules

- Cuboids normalize coordinate ordering and reject overflow or empty ranges.
- Entity sets are sorted, deduplicated, generation-checked, and scope-checked.
- Free text is length-bounded, normalized only where semantics permit, and never interpreted as
  executable code.
- Enumerations are closed. Unknown values fail compatibility checks rather than falling through.
- Preconditions and postconditions are sorted by canonical predicate encoding.
- Dependencies form a finite acyclic graph.
- Compensation actions are compiled and authorized independently.

## Risk elevation examples

The registry minimum is a floor. Context raises risk when, for example:

- a dig designation intersects an aquifer, magma, cavern boundary, support-critical tile, or
  protected region;
- construction consumes a unique artifact or the fortress's last critical resource;
- a labor change removes the only available diagnostician, broker, manager, or military command;
- a stockpile change can expose dangerous materials or sever critical logistics;
- a military action deploys civilians or opens a sealed threat boundary;
- a restore would discard uncheckpointed verified work.

## Extension admission

An extension action cannot enter the production registry until it has:

1. a stable identifier and schema;
2. a minimum risk classification;
3. a capability and scope mapper;
4. deterministic normalization and plan hashing;
5. bridge compatibility probes;
6. semantic postconditions;
7. idempotency/reconciliation behavior;
8. failure and cancellation matrices;
9. negative-evidence artifacts.
