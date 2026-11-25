# Stop production clocks when an order can no longer be read

OrderRun/1.14 verifies the raw safety pause against the original fortress even
when its manager-order queue becomes unreadable. Cancellation and plugin drain
use the same independent clock path. Each ordinary running callback checks the
clock and the predicate against a single validated order sample.
