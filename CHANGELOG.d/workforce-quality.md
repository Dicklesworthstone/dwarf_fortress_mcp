### Source: quality-aware workforce allocation backend

Added a bounded, standard-library-only lexicographic minimum-cost maximum-flow
solver with an independent residual/cut certificate verifier. The model maximizes
headcount, then role priority, effective skill, nominal skill and inverse travel
cost without double-booking workers. Includes eight Rust tests and a Python
reference with 4608 exhaustive-oracle comparisons. Rust tests are unexecuted;
no native/live/production qualification is claimed. See docs/WORKFORCE_QUALITY.md.
