# Concurrent unit and fortress configuration in the laboratory

Two members of a shared laboratory fortress can prepare work against the same
unit. When one member changes `labor.BREW`, the other member's sealed
`labor.MINE` or `burrow.<id>` setter can now survive the whole-entity revision
conflict. The host checks a bounded field witness, recompiles the original
semantic action request at the actual current anchor, and commits only if the
entire reviewed execution contract remains identical.

Ordinary stockpile and standing-order requests have a second closed refinement.
Members can change different capacities on one stockpile or different
fortress-wide order keys without losing each other's settings. It can also
reuse the canonical settings entity when another member created it first.
The host accepts a shared written field only when its source fact is unchanged
or its current qualified value already equals the exact requested value.

The receipt carries a deterministic `witness_rebase` certificate with schema
`dfmcp.field-witness-rebase/1`. It binds both plan digests and source state hashes,
the exact identity and generation of each read entity, its old and current
revision, each read field's original fact digest or structural absence, and
whether the field belongs to the write footprint. It retains the original
coarse conflict that the field proof resolved. A retry using the original
reviewed digest returns the same committed action receipts through the normal
idempotency path.

## Unit-field refinement boundary

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

## Concurrent stockpile and standing-order settings

The configuration path is limited to raw action sources containing only
`configure_stockpile` and `set_standing_order`. Every action must be immediate,
without caller guards, compensation or temporal obligations. The original
goal and each step's postconditions must contain only positive equality and
kind results established by the reviewed configuration writes. Arbitrary
boolean, spatial, relation, aggregate and owned-history predicates remain on
the explicit reviewed replay path. Mixed action families do too.

For a stockpile, `accepts` is one complete sorted set. Each supplied
`max_bins`, `max_barrels` or `max_wheelbarrows` value is a separate exact write;
an omitted capacity is untouched. For a standing order, only its exact
`standing_order.<key>` field is written. The proof constructs the sorted
write footprint from the real actions and compares the retained base fact,
current fact and requested value for every field:

| Current write field | Decision |
| --- | --- |
| Complete fact unchanged from the base, including structural absence | Replay the exact reviewed write. |
| Fact changed, but its qualified known value equals the exact requested value | Preserve the converged value while replaying the remaining writes. |
| Fact changed to a different value, became unavailable, or lost qualified provenance | Require explicit review; dispatch no effect automatically. |

For example, two members can request `accepts:["FOOD"]` together while one
sets bins to 7 and the other sets barrels to 8. After the first commit, the
second proof records that the filters converged, its barrel source fact
remained unchanged, and the current bin field will be preserved. A competing
bin value or a different filter set requires review. Filter sets are never
unioned. The unchanged original action source is replayed against the actual
current snapshot, so every omitted field survives through the ordinary
reference effect. No synthesized merged world replaces current observations.

The current stockpile source also seals the full requested goal, including
all supplied capacities and the exact stockpile kind; see
[complete stockpile settings](LAB_SEMANTIC_ACTIONS.md#complete-stockpile-settings).
A partially converged request remains actionable until its remaining
settings are satisfied. If every original goal already holds, the ordinary
planner can refuse a redundant replay; the refinement does not fabricate a
commit receipt. Historical stockpile sources retain their exact historical
proof interpretation during reconstruction.

Stockpile targets retain their exact identity, nonzero generation and kind.
Existing fortress settings do too. Revisions cannot regress or be exhausted,
and a changed written fact requires an advancing revision. A previously
missing settings entity may be reused only at the deterministic identity for
this fortress, with the exact `fortress_settings` kind, generation 1 and a
bounded complete set of qualified known standing-order fields. This is the
registered standing-order creation rule; it permits no arbitrary entity
insertion, deletion or replacement.

Every written field's source facts retain their provenance, source digest,
observation tick, compatibility value and presence marker. Missing fields
remain distinct from explicit `Absent`. Unknown, unsupported, omitted,
redacted, stale, future-dated and inconsistent facts cannot be treated as
converged settings. Filter values are bounded canonical text sets, capacities
are unsigned 32-bit values, and standing-order values are bounded nonempty
text. Repeated identical writes may share a field; ordered different writes
to one field require review. The complete entity-and-edge absence census for
optional laboratory models applies to this path as well.

The resulting `witness_rebase` certificate uses schema
`dfmcp.configuration-merge/1`. It binds both source state hashes, the original
and replayed plan digests, exact identity and generation information,
original and current fact digests or structural absence, typed requested
values, owning step numbers, the decision for each write and the original
coarse conflict. Its deterministic policy preserves current unwritten fields,
accepts unchanged source facts or exact convergence, and refuses divergence.
The existing unit-field certificate representation is unchanged.

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

The configuration branch uses the same action, identity, field, model-census,
operation, byte and wall-time limits. It admits at most 64 filter tokens per
stockpile. Its certificate is a compatibility proof only: ordinary original
source replay must still preserve the entire reviewed execution contract,
and current capabilities, risk grants, checkpoint requirements, leases and
normal preparation and commit admission still apply.

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

Scoped Rustfmt passes for this session's changed source. All 25 tests in the
complete production witness module and its original field-refinement includes
passed with real pinned dependencies, including all six new relation/aggregate
component cases and both endpoint safety regressions. No implementation copy,
substitute type or omitted child test was used.

The 11 new guarded-setter parser/public-facade tests remain unexecuted. Complete
MCP test and typecheck attempts, including a bounded codegen-partition retry,
were SIGKILLed in unchanged Asupersync under observed memory pressure before
MCP diagnostics or execution. The component results do not establish a
successful full MCP build, real stdio execution or native admission.

`configuration_witness_tests.rs` exercises the actual planner and reference
effects for independent capacities, exact filter convergence, partially
converged goals, divergent writes, complete standing-order entity creation,
missing versus explicit absence, invalid provenance and future facts,
generation/revision changes, unsupported execution contracts, model markers
on entities and edges, bounded work and deterministic certificates.
`configuration_witness_mcp_tests.rs` supplies four public-facade regressions
for shared stockpile updates, simultaneous standing-order creation, explicit
review of conflicts, revoked current authority and original-digest retry.
Those facade regressions have not executed because full MCP compilation is
still blocked in unchanged Asupersync; component tests do not substitute for
that integration check.
