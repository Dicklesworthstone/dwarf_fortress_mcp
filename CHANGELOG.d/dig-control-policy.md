### Bind mining control to live leases, protected blocks and exact review

Add core verification of actual exclusive spatial lease records and a reusable
mining policy guard. Check the entire shared-block write footprint, current
capabilities, exact source/session/journal, canonical protected regions and a
consumed, policy-bound review seal at commit. Required-checkpoint policy fails
closed; only an explicit trusted disposable-fortress policy allows uncheckpointed
development designation. Query/retirement remain separate from new-work permission.

Ten Rust regression tests are registered but uncompiled/unexecuted. No native
wire, dependencies, recovery profile or production admission changes. See
`docs/DIG_CONTROL_POLICY.md`.
