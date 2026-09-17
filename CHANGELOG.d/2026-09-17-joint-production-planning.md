# Bounded joint production quota planning

- Replace the existing single-quota compiler's unbounded recursive expansion with
  one iterative, work-bounded solver shared by single and simultaneous quotas.
- Credit initial stock once across competing goals, preserve final-stock quotas
  consumed by other recipes, aggregate shared supplier demand before rounding,
  and return complete material balances and raw-resource shortages.
- Expose hypothetical supplier-first steps with explicit dependency indices;
  refuse all action conversion when any modeled resource is short. Preserve the
  existing catalog and action-threshold semantics as illustrative proposals.
- Add independent resource/edge/order/work limits, reachable-model validation,
  checked arithmetic and active-cycle refusal without recursive stack growth.
- Register thirteen Rust regression functions, including an exhaustive 4,096-model
  oracle. Rust/Cargo/rustfmt are absent, so no Rust test or qualification pass is
  claimed. The independent Python reference passed 4,105 model cases; this does
  not execute production Rust, MCP or DFHack.

Implementation and evidence limits are in `docs/JOINT_PRODUCTION_PLANNING.md`.
The overall phase remains unadmitted development. No native method, dependency,
production runner, compatibility registry or mutation authority is changed.
