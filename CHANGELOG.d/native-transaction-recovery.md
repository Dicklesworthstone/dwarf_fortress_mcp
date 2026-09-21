### Immediate, fenced recovery for native job and work-order mutations

Job suspension now attempts fresh readback even when its single setter throws.
Only the exact pre-write job context, requested flag and reserved mutation
sequence can establish Applied or NotApplied. An intervening sequence change,
unavailable readback or changed context remains permanently Unknown.

Work-order creation likewise verifies the exact expected queue transition and
native configuration after an insertion exception. Queue/clock/sequence readback
also brackets the configuration verifier. Only complete Created evidence releases
the unresolved-creation guard; partial insertion, unchanged queues and unavailable
or mismatched evidence remain Unknown and block further creation. Neither path
retries a setter, rolls back native work, or promotes a later query to success.

Run the production-header regression suites (111 deterministic cases):

```sh
python3 scripts/check_native_transaction_recovery.py
python3 scripts/check_native_transaction_recovery.py --sanitize
```

Both commands passed with GCC, including AddressSanitizer/UndefinedBehaviorSanitizer.
Both pre-change engines fail the new exception-recovery regressions. These checks
use native callback fixtures, not DFHack: Rust compilation, full qualification,
the real SDK/plugin and live-game behavior remain unverified by this change.
No wire format, compatibility admission or production-runner policy changed.
