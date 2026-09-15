# Durable spatial capture history and exact-record analysis

- Generalize the existing synced observation journal through sealed fixed codecs
  for operations/1.3, operations/1.4 and coherent spatial/1.6. Preserve legacy 1.3
  APIs, file layout and digest domains; reject cross-profile files before repair.
- Integrate the optional spatial journal into native bootstrap and refresh, with
  sync before publication and exact replay of combined state and generations.
- Add spatial `history` and `historical_query` through the existing MCP tool,
  including archived candidate routes and route-aware allocation. Route
  drill-downs stay on their exact archive record. Current authority and watches
  remain current; historical reads perform no live refresh or state mutation.
- Separate acquisition/replay and response budgets and retain whole-row output
  within the complete Agent Turn. Valid archive reads remain available after
  native source failure. Paths and explicit incomplete-tail repair are
  operator-only environment configuration.
- Register fourteen new Rust scenarios. Rust compilation/tests, Clippy, stdio,
  filesystem crash behavior and live DFHack execution remain unverified. The new
  schema's metadata and 50 wrapper/gate component cases passed, as did sixteen
  independent JSON-length checks; composed schema and Rust execution are not
  implied by those checks.
- No 1.4 MCP journal wiring, durable watches/baselines, offline bootstrap,
  automatic retention rotation, native bridge change, dependency change, effect
  authority or production admission is introduced.

See `docs/SPATIAL_HISTORY.md` for setup, examples and remaining limits.
