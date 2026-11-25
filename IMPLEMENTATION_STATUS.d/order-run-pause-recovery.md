# OrderRun safety-stop recovery

A failed order observation now retains a native-failure trigger and allows an
independent fortress-bound clock readback after the safety setter. Cancellation
and unload can finish while the order queue remains unreadable. No order
completion follows from a verified pause. Capture, plan and receipt formats
keep their existing identities. The native plugin regression covers a malformed
queue during running, cancellation and unload.

Work item: df-combined-order-clock-c8m. Live game acceptance remains open.
