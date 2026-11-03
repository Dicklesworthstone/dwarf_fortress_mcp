# Cache-bearing Manager 1.23 development successor

Beads `.14.4` and `.14.2` own this explicitly unadmitted successor. The real U
MCP trial proved the 1.22 assignment/HF/history projection, while V observed an
empty `historical_entity.assignments_by_type[MANAGE_PRODUCTION]` despite both
DFHack getters identifying the Manager. Work orders still requested a Manager.
The attempted ordinary-UI reappointment comparison did not complete. This is a
source-backed missing-cache diagnosis; only a new real trial can prove its effect
on normal production. Historical 1.22 proof is not a functional-manager claim.

The 1.22 plugin, native vectors and default `manager` CLI remain unchanged.
Manager 1.23 uses package `dfmcp.manager.v1_23`, plugin `dfmcp_manager_v1_23`,
the same six fixed method names, and the frozen eleven MCP tool names. Start
the dedicated `dfmcp-manager-v1-23-dev-server` or the standard CLI with
`serve-gameplay-dev manager-v1-23`. The public wrapper rejects production
protocol state before the shared runtime; other production provenance and every
foreign `DFMCP_*` variable are rejected before service. A private typed startup
pin selects the generation once; no MCP argument selects a protocol or method.

Host names are `DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23=1`,
`DFMCP_MANAGER_V1_23_TOKEN`, `DFMCP_MANAGER_V1_23_ALLOW_APPOINT=1`, and
`DFMCP_MANAGER_V1_23_MCP_CONFIG`. Configuration has the same closed source,
fortress/citizen/HF, authority, lease, custody and checkpoint fields as 1.22,
with schema `dfmcp.manager-appointment-mcp-development/2`. The role remains
MANAGER and the candidate remains the current Expedition Leader.

Canonical capture, plan, native record, insertion, binding, original intent,
journal/header/frame and ACK use distinct `023` magic. A 1.23 Entity retains
all legacy fields, then appends a big-endian u16 role-bucket count; each ordered
bucket has a u16 entry count followed by ordered signed i32 assignment IDs.
Both group and civ are captured. Limits are 128 buckets, 256 entries per bucket
and 4096 entries across both entities. Null/foreign/duplicate membership is
rejected; pointers never cross the bridge. Bucket order and entry order are
preserved. Group and civ have the same role domain.

The native source accepts only the ordinary Manager definition with exactly
MANAGE_PRODUCTION (responsibility 4). Preparation requires both production
buckets empty, the existing vacant assignment and all original 1.22 eligibility
conditions. The exact witness includes every captured bucket. The intended
write adds the same already verified assignment to the initially empty group
production bucket, preserving every other group bucket and the entire civ
projection. Vector capacity is reserved before semantic writes; revalidation
precedes the assignment/HF/history/cache transaction under CoreSuspender.
Two fresh captures and exact expected-poststate comparison precede success.
Post-writer failure stays indeterminate; same-key replay cannot invoke a writer.

Preparation reserves room for the new HF link, responsibility-cache entry and
history event: at most 255 HF links, 4095 total cache entries and 65535 history
events before the write. Terminal observation accepts the corresponding maxima
of 256, 4096 and 65536. The legacy 1.22 native limits remain unchanged. Boundary
regressions exercise the last eligible append, full refusal before writing and
oversized observations; compiled mutants restore the faulty limits.

Custody filenames are `manager-appointment-v1-23.journal` and
`manager-appointment-v1-23.ack`; old files cannot be adopted or interpreted as
new preparation. Review, 60-second shrinking deadlines, one-use local permission,
write-ahead independent ACK, reconciliation and offline retention remain required.
`result.original.production_manager_cache_proved` is true only for a verified
1.23 terminal record. Legacy 1.22 exposes no observed role-cache rows and cannot
prove this predicate. Empty legacy data must not be interpreted as absence.

Native SDK-double checks and a matching-SDK development build exist for the
preceding cache source. Clean `e899a442` passed 23 adapter cases and actual modern
MCP suites of 8 dedicated 1.23, 8 CLI 1.23 and 7 legacy cases. Native peers in those
suites are doubles. Its test-only import error and the later strict-Clippy test
diagnostics were fixed forward. These results do not qualify the newer append
boundary source. Live production, full current-source qualification and admission
remain pending. SDK-double output is not game evidence.
The registry and production protocol map remain unchanged.
