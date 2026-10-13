# GA-FLOW observed route resilience — source present

The bounded terrain reference now computes maximum edge-disjoint or internally
vertex-disjoint candidate routes and verifies an equal-cardinality separating
cut. Adjacent endpoints expose their unavoidable direct-edge cut explicitly;
excluded endpoints never become a certified zero count. This increment is an
implementation of observed graph semantics, not native safety or throughput.

The independent Python design oracle passed 42,856 graph/map/model cases and
adversarial examples. Fifteen Rust tests are added but have not been executed;
there is no Rust toolchain or DF/DFHack runtime in the authoring environment.
No full-workspace, native, live, registry, floor, artifact or runtime admission
is claimed. The graph integration bead `df-franken-storage-mvcc-54h.4` stays open.
See `docs/MAP_ROUTE_RESILIENCE.md` for the precise scope and validation commands.
