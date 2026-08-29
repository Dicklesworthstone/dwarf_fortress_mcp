# Capability registry

Capabilities are data carried by every operation. Possession of an MCP connection, process handle,
or bridge socket grants no ambient authority. This table matches the initial registry frozen in the
comprehensive plan.

## Capability vocabulary

| Capability | Minimum risk | Typical scopes | Notes |
|---|---:|---|---|
| `observe` | read-only | fortress, map cuboid, entity kinds, fields | Read canonical state and deltas. |
| `query` | read-only | query namespaces, entity kinds, fields | Execute bounded DfQL. |
| `plan` | read-only | action families and target scopes | Compile only; grants no mutation. |
| `designate` | reversible | spatial cuboids | Dig/chop/gather/traffic-style designations. |
| `construct` | guarded | spatial cuboids, building classes | May consume scarce materials and block tiles. |
| `configure_labor` | reversible | citizen sets, labor kinds | Must preserve protected roles and policies. |
| `configure_production` | reversible | workshops, work orders, products | Temporal production requires obligations. |
| `configure_logistics` | reversible | stockpiles, burrows, routes, standing orders | Context can elevate risk. |
| `configure_military` | guarded | squads, alerts, schedules, equipment | Threat-facing changes require stricter policy. |
| `control_clock` | reversible | fortress clock | Requires an exclusive clock lease. |
| `checkpoint` | guarded | fortress/save slot | Produces durable recovery evidence. |
| `restore` | guarded | named checkpoint | Starts a new observation epoch. |
| `extension` | registry-defined | named extension/version | Only registered typed extensions. |
| `diagnostic_raw` | read-only | allowlisted raw field groups | Tainted, non-authoritative diagnostic access. |
| `doctor` | read-only | session/bridge/ledger/checkpoints | Runs integrity checks without mutation. |
| `repair_plan` | guarded | offline repair namespace | Produces a sealed plan but does not apply it. |
| `repair_apply` | irreversible | exact sealed repair plan | Offline/local policy only by default. |
| `admin` | irreversible ceiling | explicitly scoped administration | Super-capability; still obeys scopes, leases, budgets, and invariants. |

## Scope algebra

A grant is the intersection of all supplied restrictions:

- subject and delegation parent;
- fortress identity;
- entity kinds and explicit entity IDs/generations;
- one or more spatial cuboids;
- action and resource/configuration domains;
- game-tick expiry;
- remaining-use count;
- maximum risk tier;
- multidimensional work budget;
- policy version.

Empty scope components mean unrestricted **within the grant**, not globally. A delegated grant is
valid only if every dimension is equal to or narrower than its parent.

## Enforcement points

1. MCP intake and session admission;
2. query planning;
3. intent compilation;
4. plan preparation;
5. lease acquisition;
6. checkpoint policy;
7. immediate pre-dispatch revalidation;
8. bridge allowlist validation;
9. compensation and restore;
10. repair planning/application.

A capability denial is deterministic and carries the missing capability and rejected scope. It is
not retryable unless authority or state changes.
