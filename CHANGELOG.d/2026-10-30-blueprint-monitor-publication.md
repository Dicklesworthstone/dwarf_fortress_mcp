## Withdraw unfinished monitor continuity after failed MCP publication

Apply monitor continuity withdrawal at failed request, runtime, authority and
response boundaries. Track successful owned-worker acknowledgement so a missing
outer-runtime result cannot carry an unfinished streak into the next request.
Preserve recovery identity and label retained target pages historical after
source invalidation. Existing monitor-only recovery remains unchanged.

Two handler regression groups, one runtime test and strengthened source-drift
assertions accompany the change. Query-schema validation passes 49 valid and
175 invalid cases; new Rust execution remains unverified after compiler SIGKILL
and filesystem exhaustion. Full runtime/native/live qualification is unclaimed.
