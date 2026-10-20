# Discover retained construction-clock work without transcript memory

Added offline `inventory` to the existing scoped-clock recovery CLI. It validates
all original scope/intent/receipt groups in one private directory, returns bounded
query-bound pages and separates pending work, retained terminal receipts and
unresolved source loss. Sequential readers share one lock through final output.
No batch reconstruction, native call, repair, restart or permission change occurs.
All 50 component tests and 13 assertion mutation checks pass; no live qualification.
