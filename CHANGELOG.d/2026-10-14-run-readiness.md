## Construction-aware bounded-run stops

- Separate local prefix eligibility from observed route/construction readiness.
- Return blocked reviews without consent and preserve completed outcomes plus
  exact blocking evidence when later construction is pending.
- Preserve progress after failed later reviews without reusing cached readiness.
- Require fresh review after any run; old inventory-bound consent never resumes.
- 27 owner tests pass (12 new), with explicit native-step/custody ports. No native,
  private-journal, Rust/MCP or production-admission qualification claimed.
