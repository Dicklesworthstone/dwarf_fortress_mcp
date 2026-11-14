# Concurrent unit setters in the laboratory

Two members of a shared laboratory fortress can prepare work against the same
unit. When one member changes `labor.BREW`, the other member's sealed
`labor.MINE` or `burrow.<id>` setter can now survive the whole-entity revision
conflict. The host checks a bounded field witness, recompiles the original
semantic action request at the actual current anchor, and commits only if the
entire reviewed execution contract remains identical.

The receipt carries a deterministic `witness_rebase` certificate with schema
`dfmcp.field-witness-rebase/1`. It binds both plan digests and source state hashes,
the exact identity and generation of each read entity, its old and current
revision, each read field's original fact digest or structural absence, and
whether the field belongs to the write footprint. It retains the original
coarse conflict that the field proof resolved. A retry using the original
reviewed digest returns the same committed action receipts through the normal
idempotency path.

## Closed implementation boundary

This refinement runs only after the ordinary whole-record witness reports a
conflict. The original source must be raw semantic `actions`; every action and
compensation must be `set_labor` or `set_burrow_membership`. The original and
current snapshots must belong to the same fortress, epoch and game tick, with a
strictly advancing observation sequence. Temporal obligations are ineligible.

The reference effects require the exact unit kind and, for membership, the
exact burrow kind. The witness preserves those identities, generations and
kinds and rejects regressing or exhausted revisions. It reads every explicit
terminal, precondition and postcondition branch, including all `any` branches
and negation. It also reads the prior value of every field written by an action
or compensation. A changed generation, prior write fact, explicit field read,
pause read, or entity presence causes a refusal.

Field comparisons preserve the complete original fact, including source,
source digest, observation tick, compatibility value and presence marker.
Missing map entries are compared as structural absence, without interpreting
them as `false`; explicit `Absent` remains distinct. Unknown, stale, omitted,
unsupported and redacted facts do not qualify for refinement. Present facts
must carry eligible laboratory provenance, and values and predicate literals
must be bounded scalars.

Every reference effect first validates the optional shared-workforce,
construction-material, provisions and finite-harvest models. Consequently this
refinement requires a complete bounded census proving the absence of those
model declarations and task-history markers on **both entities and edges**.
A retained harvest-task identity also prevents refinement when its fields are
missing. Production markers are conservatively refused even for unrelated recipes.
Opt-in model worlds continue through explicit source replay and review. The
absence census is part of the certificate; an incomplete scan cannot produce
an accepted field proof.

Exact `EdgeExists` and absolute `EntityQuantity` predicates are also eligible
within this closed setter boundary. Caller-supplied guards reach these existing
typed predicates through the optional `preconditions` array in raw action
requests; see [the request grammar](LAB_SEMANTIC_ACTIONS.md#caller-guards-for-labor-and-burrow-setters).
The planner seals and rechecks them using its existing laboratory evidence
policy.

### Exact relation reads

A present edge must retain its complete identity, revision, kind, endpoints and
qualified field facts. The certificate includes a deterministic full-record
commitment and binds both endpoint entity identities, generations and kinds.
Unrelated endpoint fields may change only after the ordinary coarse endpoint
conflict reaches this finer check. Edge insertion, removal, changed edge
content, dangling endpoints or endpoint generation reuse require fresh review.
A negative edge read preserves the absence of that exact identity in both
complete laboratory graphs. It does not establish absence of an entire
relation family or arbitrary adjacency range.

### Absolute quantity reads

The proof scans the entire canonical entity-kind domain in each version,
including reserved-name aliases and entities whose selector currently does
not match. It requires exactly the same member identities and generations.
Every member's selector fact is retained; each matching member's unsigned
quantity fact is retained when the predicate sums a field. Without a quantity
field, each match contributes one. Unrelated fields, including the quantity
of a nonmatching member, may change.

A matching subtotal is insufficient when any selector or contributing quantity
is unavailable. Unsigned overflow also refuses refinement. Insertions, removals,
selector changes and substituted contributors remain conflicts even if the
total is unchanged or the new member does not match. The same rule protects an
initially empty domain. Every `any` and `not` branch remains witnessed; a
currently true sibling cannot erase an unknown aggregate read.

Optional `edges` and `aggregate_domains` certificate sections record exact
edge commitments, complete membership, matched contributors, exact total and
the canonical aggregate predicate digest. The ordinary field section retains
the full selector and quantity fact digests. Certificates that use neither new
read family retain their existing representation.

Range, spatial, temporal quantity-increase and owned-history predicates remain
on the conservative replay path. Blueprint and objective compilers are also
excluded because they can read domains that their resulting action list does
not name.
Ordinary whole-record relation witnesses also compare both complete endpoint
entity records after checking the edge itself. A byte-identical relation cannot
hide endpoint removal, generation reuse, kind drift or another endpoint-record
change. Relation certificates declare `edge_endpoint_records: true`; witnesses
without relation reads retain their exact prior certificate representation.

## Bounds and authority

The proof admits at most 64 steps, 256 entity identities, 256 distinct fields,
256 exact edge reads, 64 aggregate predicates, predicate depth 32, 4,096
entity-plus-edge records per version, 262,144 counted operations and 1 MiB of
charged scalar data. Aggregate members and relation endpoints share the same
256-identity limit; selectors and quantities share the same 256-field limit. The negotiated action, entity and
byte budgets can tighten those limits. Wall time is capped by the smaller of
the negotiated budget and 100 ms. Exhaustion returns a refinement refusal and
the ordinary reviewed replay; it does not dispatch effects.

The current-source planner rechecks hard preconditions. Automatic replay must
preserve the summary, terminal goal, expiry, risk, required capabilities,
checkpoint requirement and every step's action, guards, postconditions,
compensation, dependencies and obligation. Current capability grants, scope,
lease acquisition, fresh preparation and normal commit admission still apply.
The certificate proves only this bounded read/write compatibility. It grants
no authority and does not create or publish a merged synthetic world.

This is a deterministic laboratory feature. It does not admit a native bridge,
implement arbitrary structural three-way merge, or establish general action
commutativity.

## Focused verification

`field_witness_tests.rs` exercises the actual planner, canonical snapshots and
reference effects for same-unit labor and burrow concurrency, deterministic
certificates, exact prior facts and provenance, unavailable presence, boolean
branches, compensation, entity replacement, lineage, implicit model reads and
budget refusals. `field_witness_mcp_tests.rs` covers the public shared-session
plan/commit/query/retry path, revoked current effect authority and incomplete
census refusal. The existing public conflict test now changes the exact same
labor field, preserving explicit replan coverage after independent fields
became eligible for automatic replay.

The guarded-setter additions include parser and public-facade regression
sources for positive and negative relation guards, shared-session aggregate
guards, initial false/unknown/negated-unknown refusals, unchanged-total
substitutions, nonmatching and alias phantoms, empty domains, endpoint reuse,
every boolean branch, overflow and current authority. Adversarial public
fixtures inject independent canonical observations; their plan and commit
requests still pass caller JSON through the ordinary facade.

Scoped Rustfmt completed after the execution environment returned. The full
MCP test-library build was SIGKILLed in unchanged Asupersync under memory
pressure without a Rust source diagnostic; a lower-memory actual-source
typecheck is pending. The guarded-setter regressions remain unexecuted.
Fourteen laboratory effect-drain tests, including the three new emergency-pause
cases, passed; those checks do not qualify this MCP feature. Source review and
partial component checks do not establish a passing MCP build, real stdio
execution or native admission.
