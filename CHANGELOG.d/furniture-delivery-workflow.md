## Delivery-aware review creation

- Wire the shared delivery-sequence compiler into review creation behind explicit
  --sequence-deliveries opt-in; preserve default behavior and one acquisition.
- Carry the effective request through retained review and source-bound output.
  Reject reordering during check and export no review for blocked sequences.
- Preserve output/export work reservations and final current-authority checks.
- Forty Python component/workflow tests pass, including fourteen new cases;
  four weakened variants fail assertions. Native acquisition/budget/outer packet
  are explicit test seams, not native TCP, private batch, Rust or live evidence.
