## Discover and recover original construction clocks without remembered confirmations

- Add offline `route_furniture_progress.py inventory` with bounded, source-bound
  pages and exact original query handles. Preserve full original batch output,
  active unresolved work, one output attempt and clock/batch custody through it.
- Gate every new progress effect on complete prior scoped-clock history under
  the same directory lock. Unknown, foreign, unbound, orphan-scope and SourceLost
  effects cannot be bypassed by fresh consent or a filtered inventory page.
- Recheck prior files at PrepareRun, CommitRun and result boundaries. Existing
  exact-key query, native wire, scope/intent/receipt formats and grants remain.
- Forty actual Python tests and eight assertion-rejected mutants pass. Real clock
  TCP/private files execute; original route/batch/packet boundaries are test ports.
