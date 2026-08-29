# Determinism registry

Replay equality is defined over canonical input plus a complete transcript of effects and scheduler
decisions. The following domains must be captured or deliberately normalized.

| Domain | Capture/normalization rule | Equality target |
|---|---|---|
| protocol negotiation | exact offered/selected versions and features | same compatibility mode |
| canonical world input | fortress ID, epoch, cursor, tick, state hash, covered bytes | same anchor and projections |
| bridge input | exact normalized frames after validation | same adapter decisions |
| planner registry | action/schema/compatibility registry digests | same plan and risk classification |
| capabilities | ordered grants, scopes, expiries, remaining uses, budgets | same authorization result |
| clocks | named domain and sampled values | same deadline transitions |
| randomness | stream IDs, seed, draw count | same generated IDs/choices |
| scheduler | runnable-set digest and selected task | same transition trace |
| storage | ordered transaction results and durable sequence | same ledger state |
| filesystem | injected metadata/content results, not host directory order | same checkpoint/repair decisions |
| search | index version, corpus anchor, deterministic scoring config | same ordered hits and score ledger |
| text parsing | source digest, parser version, byte spans | same semantic document |
| errors | stable code plus canonical structured details | same error class and fields |
| output | canonical collection order and numeric representation | byte-stable canonical encoding |

## Forbidden hidden inputs

- ambient wall clock in core logic;
- process-global randomness;
- pointer values or allocation order;
- host hash-map iteration order;
- locale-dependent comparison or formatting;
- unrecorded environment variables;
- thread timing used as a correctness signal;
- filesystem enumeration order;
- mutable registry data without a digest;
- unspecified floating-point reduction order.

## Replay bundle minimum

A replay bundle contains protocol/schema digests, compatibility manifest, initial canonical anchor,
relevant snapshot or checkpoint reference, request stream, effect transcript, scheduler trace,
ledger segment proofs, and expected terminal evidence. Sensitive fields may be encrypted or replaced
with content-addressed redacted blobs, but omissions must be explicit.
