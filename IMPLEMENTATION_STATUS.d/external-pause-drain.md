# External pause during clock recovery

Rust and Python receipt readers for Run/1.13, OrderRun/1.14 and ExcavationRun/1.18
accept the native stopping state during external pause verification. A pending
stop retains its unverified flag. Its successor can record a verified pause or
a source change. The existing plan, operation identity and wire format apply.

Work items include df-combined-order-clock-c8m and df-cancellation-progress-certificates-vvw.
