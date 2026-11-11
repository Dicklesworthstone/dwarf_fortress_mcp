# Compact complete plan reviews

Resource-room, resource-production and ordinary production-reserve continuation
reviews retain every step, postcondition and mandatory obligation in the same
response. An optional second predicate encoding removes repeated JSON syntax
when the complete original response exceeds its negotiated output allowance.
It does not change the typed plan, source generation, digest, archive, action
authority, capability requirements, deadlines or commit behavior.

The publisher first builds and validates the complete generation-one packet.
If it already fits, its predicate representation is unchanged. Otherwise it
tries the lossless table representation below, accepting it only when the
serialized response becomes smaller and its inverse equals the entire original
response. This happens before the existing projection of archived objective
artifacts. Current work and goal truth remain complete. If the final complete
packet still exceeds its allowance, publication refuses before replacing the
pending plan; it never silently raises the allowance.

## Format

The outer review schema stays unchanged. The predicate encoding declares:

```json
{
  "schema": "dfmcp.plan-predicate-references/2",
  "scope": "same_response",
  "plan_digest": "<unchanged plan digest>",
  "values_pointer": "/predicates",
  "complete": true,
  "projection": "readable_predicate_json",
  "exact_typed_contract": "retained_plan_archive",
  "order": "terminal_then_steps_postconditions_then_obligation_children_first",
  "reference": "zero_based_predicate_index_or_boolean",
  "row_format": "kind_then_values_in_declared_column_order",
  "columns": {
    "entity_exists": ["entity_id"],
    "field": ["entity_id", "field", "op", "value"],
    "all": ["children"]
  },
  "leaf_values": "literal_not_references"
}
```

`columns` includes exactly the kinds present in the table. Each row begins
with its kind and then gives values in the declared order. For example:

```json
{
  "predicates": [
    ["entity_exists", "207"],
    ["field", "207", "construction_stage", "eq", "complete"],
    ["all", [0, 1]]
  ],
  "terminal_condition": 2
}
```

This is a format excerpt, not a complete Plan response. Terminal index `2`
restores the full conjunction of entity existence and construction completion.

A reference is either a JSON Boolean literal or an unsigned integer table
index. Only these positions interpret references: the top-level
`terminal_condition`, each element of step `postconditions`, an obligation's
`terminal`, and children of `all`, `any` and `not`. A compound table row may
reference only earlier rows, making cycles impossible. Roots may refer to
any existing row. Negative numbers, strings, fractional indices, noncanonical
references, dangling references and extra row values are refused.

Leaf values remain literal JSON, including strings resembling JSON pointers,
objects containing `$ref`, integer coordinates, entity identifiers, arrays
and nulls. The codec never recursively searches arbitrary response data for
references. Source requests, unrelated response fields and anchor encoding
retain their original values.

## Registered columns

| Kind | Values after the kind |
| --- | --- |
| `all`, `any` | `children` |
| `not` | `child` |
| `entity_exists` | `entity_id` |
| `paused` | `paused` |
| `entity_kind` | `entity_id`, `kind` |
| `field` | `entity_id`, `field`, `op`, `value` |
| `entity_quantity` | `kind`, `filter_field`, `filter_value`, `quantity_field`, `op`, `value` |
| `entity_quantity_increase` | `kind`, `filter_field`, `filter_value`, `quantity_field`, `baseline`, `increase`, `target`, `baseline_anchor`, `baseline_source_digest`, `source_generation`, `baseline_source_counters`, `meaning` |
| `edge_exists` | `edge_id`, `kind` |
| `region_terrain` | `min`, `max`, `tile` |
| `laboratory_room_access`, `laboratory_room_access_paired_stairs` | `policy`, `region`, `entry`, `targets` |
| `laboratory_civilian_release_safety` | `policy`, `burrow`, `civilians` |

`entity_exists` and `paused` restore their scalar readable forms. Other leaf
kinds restore objects with exactly the registered fields. Column declarations
are checked against the fixed renderer contract; they cannot redefine the
meaning or order of values.

## Validation and continuity

Encoding operates on a separate candidate. It validates the original shared
proof, checks the decoded candidate with the existing bounded graph validator
and requires exact JSON-value equality with the original before publishing.
Failure leaves the input intact. Decoding first reconstructs the shared DAG;
it does not eagerly expand an unchecked predicate tree.

The existing limits remain: at most 64 steps, 16,384 predicate nodes, depth
64 and the 4 MiB archive-sized expansion allowance. The compact decoder also
bounds the serialized table, step projection and reconstructed DAG before
expansion. Later-turn orientation caches expand the accepted proof into
standalone predicates, so Commit and Observe do not retain numeric references
to a previous response's table. A review is a requirement to verify, not
observed completion evidence.

## Execution qualification

New regression sources cover lossless round trips, all registered kinds and
literal value shapes, deterministic encoding, unchanged archives, malformed
references and columns, bounded expansion, complete current work, exact byte
acceptance/refusal boundaries, and standalone later-turn orientation.

The public room regression covers plain and connected two-bedroom pursuits
with existing or missing Carpenter workshops. At the default 4,096-token
allowance it requires either a complete review or an explicit budget refusal
that preserves prior pending work and world effects. With an ample allowance
it requires exact proof reconstruction and the full harvest-to-furnished-room
lifecycle, including original-goal history.

These new Rust tests were not executed. Recovered final sources were
syntax-formatted with nightly Rustfmt and verified against their local Git
blob hashes. An exact-source component build with unchanged dependency pins
was killed by SIGKILL in the unchanged dfmcp-world library before tests
compiled or ran. Executor disconnection then prevented a metadata-only check.
The implementation does not establish default-budget fit for complete room
reviews. Existing measured limits in
[LAB_RESOURCE_ROOM_PROJECTS.md](LAB_RESOURCE_ROOM_PROJECTS.md) and the runtime
evidence in [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md) remain the
qualification record. No native protocol or live admission claim changes.

Owning work: `df-lab-resource-harvest-h2m.4` and
`df-lab-resource-harvest-h2m.5`.
