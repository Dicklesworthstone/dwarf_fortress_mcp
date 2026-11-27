# Excavation clock verification

The excavation/1.18 clock owner uses a fresh source read to verify a pause triggered by a terrain capture. The callback retains the latest terrain tick while running. The native engine fixture covers tick limits, a changed site, unavailable or contradictory pause readback, and a later tick regression. These cases were source-reviewed. Native execution and a disposable-fortress campaign remain outstanding.

Owner: `dfmcp-reality-wiring-proof-4bf.5`.
