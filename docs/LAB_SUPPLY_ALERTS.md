# Food and drink alerts in the reference laboratory

The `world_alerts` in an ordinary `fortress_observe` or `fortress_wait` response identify shortages and citizen needs at the selected world anchor. For a food or drink alert, `stock` is the established inventory. `known_deprived_units` counts observed living dwarves reporting hunger or thirst. `deprived_units` gives the exact total when every relevant need is established.

Two predictions answer different planning questions.

| Field | Meaning in elapsed game ticks |
| --- | --- |
| `ticks_until_exhausted` | When consumption would take the current stock to zero. Already empty stock is at zero ticks. |
| `ticks_until_first_unserved_round` | When the next consumption round would leave one or more living dwarves without their ration. |

Both use the observed metabolism clock, the actual consumption interval for the resource, and the complete living roster. They assume no new supply, other stock changes or population changes. With seven dwarves, seven drinks, and one tick until the next drink round, the ledger would empty in one tick. All seven would drink. The first unserved round would be 1,201 ticks away. With three drinks instead, the next round would both empty the ledger and leave four dwarves unserved.

A missing or unqualified clock leaves future deadlines unknown. So does a deadline beyond the representable world or metabolism timeline. A currently observed hungry or thirsty citizen still generates a critical welfare finding if the ledger disappears. Unknown stock cannot justify a production quantity. With ample stocks, explicit unavailable need reports appear as separate `drink_need_coverage` or `food_need_coverage` findings. A new scenario with no need reports yet does not itself raise a welfare alarm.

These predictions apply to the laboratory reference metabolism. Current stock, current citizen needs and production remedies retain their existing evidence and authority requirements.
