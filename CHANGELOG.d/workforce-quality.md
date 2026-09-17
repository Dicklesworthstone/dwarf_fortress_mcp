### Source: quality-aware workforce planning

Added a bounded, standard-library-only lexicographic minimum-cost maximum-flow
solver with an independent residual/cut verifier, and integrated the opt-in
`priority_skill_distance` objective into spatial/1.8 `workforce_plan`. It maximizes
headcount, then role priority, effective skill, nominal skill and inverse travel
cost without double-booking workers. Legacy default behavior and wp1 identities
are preserved; quality requests use priority/objective-bound wq1 continuations.

Includes eight standalone Rust tests, six registered-handler tests, a Python
reference passing 4608 exhaustive-oracle comparisons, and 362 passing schema cases.
Rust tests are unexecuted; no native/live/production qualification is claimed.
See docs/WORKFORCE_QUALITY.md. No dependency, mutation or admission changes.
