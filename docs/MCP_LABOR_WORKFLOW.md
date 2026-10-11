# Staffing targets through fortress.plan

Owning bead: `dfmcp-wp-pln03-labor-civilian-alert-fsm-zdm` (WP-PLN-03).

The laboratory `fortress.plan` tool accepts a versioned staffing request inside
its existing `actions` string. No twelfth tool, direct effect endpoint or native
protocol is added. Existing JSON action arrays, blueprints, production goals,
scenarios and projections keep their original implementations.

## Request and execution

Use `schemas/examples/labor_execution_v1.json` as the `actions` string:

```json
{
  "workflow": "labor_allocation_v1",
  "roster": [
    {
      "id": "1001",
      "skills": {"MINING": 20, "BREW": 10},
      "stress_level": 100,
      "assigned_labors": ["BREW"]
    },
    {
      "id": "1002",
      "skills": {"BREW": 9},
      "stress_level": 100,
      "assigned_labors": []
    }
  ],
  "demands": [
    {"labor": "MINING", "skill": "MINING", "workers": 1, "minimum_skill": 1, "priority": 0},
    {"labor": "BREW", "skill": "BREW", "workers": 1, "minimum_skill": 1, "priority": 1}
  ]
}
```

This proposes moving the flexible worker from brewing to mining and using the
other worker for brewing. The ordinary tool returns a sealed plan and digest.
Inspect its steps and guards, commit that digest with `fortress.commit`, then
use bounded `fortress.wait` calls to observe dependent steps and their proofs.
Plan construction itself performs no effect. Negotiated Plan and ConfigureLabor
requirements, original identities, replay, source witnesses and cancellation
continue to belong to the existing server and adapter.

The initial managed labor matrix must actually be known. `assigned_labors: []`
is a *claim of false bits* for the requested managed roles, not evidence that
omitted fields mean false. The starter laboratory does not populate every labor
bit. For a reproducible reference-only example, first submit the explicit action
array in `schemas/examples/labor_execution_setup_lab.json`, commit it and wait
for verification. That file deliberately sets the example's initial values on
units 1001 and 1002. It is not a native initialization procedure or permission to
infer unknown live state. The existing brewer (1003) is not changed by this example.

A stale or false declared initial matrix refuses at ordinary semantic planning.
Missing fields also refuse. Actual initial state guards and final full-configuration
predicates come from `dfmcp_intent::labor_execution`; the MCP decoder never
implements a second allocation algorithm or directly writes a labor field.

## Semantics and limitations

Lowering refuses unmet staffing targets instead of silently committing a partial
assignment. A requested count of zero explicitly retires a managed role. New
labor assignments remain exclusive within the declared managed-role model;
other unmentioned labors and citizens are not part of that claim. Skills and
stress are advisory, caller-declared model inputs, not observed or native-qualified
facts. Minimum skill zero permits model generalists, not an inference of native
skill absence.

Every generated step depends on its predecessor: disables must be verified before
enables can start. Each write checks its prior labor value. The last action's
postconditions include the complete target, including unchanged workers. That
extra predicate is sealed even though the generic action-intent terminal is built
from changed-action defaults. A generic Task therefore still needs every step's
full proof before completing; changed bits alone cannot complete the last action.
Configuration proof does not prove that dwarves started jobs or that the staffing
model predicts native performance.

The MCP path lowers to the existing semantic action-intent handler. The standalone
`LaborExecutionPlan::prepare` embedding API additionally offers its own whole-roster
Observe and encoded-byte checks; it is not the MCP handler. The handler retains
its existing negotiated action/entity checks and output-budget fitter. Workflow
parsing has independent fixed ceilings: 16 KiB input, 64 citizens, 16 demands,
64 requested positions, 16 generated actions, one million charged work units and
256 KiB of compiled predicate/action accounting. There is no partial executable
return on a bound failure.

Unknown fields and workflow versions, duplicate profile IDs, repeated labor
assignments, duplicate skill JSON keys, repeated demand labors, noncanonical or
out-of-range IDs and malformed numeric values refuse. JSON Schema describes the
structural contract in `schemas/labor.execution.request.schema.json`; Rust also
checks byte counts, identity uniqueness, total positions, no-ops, feasibility,
work and generated-size bounds.

The existing live single-work-detail handoff remains separate and will not accept
these dependent, compensated multi-step plans without additional reviewed native
execution support. No compatibility registry entry or production runner changes.

## Evidence

Six Rust tests are present: legacy/guard/order compatibility, strict decoding,
checked-in example decoding, real-facade plan/commit/wait and replay, refusal of
missing or stale initial state, and mutation-capability denial. Together with the
compiler's seven core and two real-adapter tests, this increment has fifteen
new Rust test functions. They were source-reviewed but not compiled or executed:
Rust, Cargo and DFHack are unavailable in the editing environment.

The request schema was checked against the Draft 2020-12 metaschema, accepted the
checked-in example and rejected all 30 tested malformed shape mutations. This
is Python/JSON-Schema evidence only, not Rust decoding, MCP process, native or
live qualification. The original MCP library bytes were independently matched
to their Git blob before adding only the module-path attribute. The original
`lab_world.rs` implementation remains byte-identical.
