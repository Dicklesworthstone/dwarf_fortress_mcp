"""Whole-mask excavation capacity, not native mining eligibility or authority.

Internal leaf filter for the explicit bounded-original-dig/1 layout strategy.
Only compiler-accepted RoomPlans enter. Binary cost-one cells are assumed to
be remaining walls; the observed wrapper derives those bits from raw evidence.
The final terrain survey repeats the same partitioner on its own decoded mask.
Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from typing import Callable

from furniture_plan import canonical, require
from room_layout_costs import TileCosts
from room_provisioning import RoomPlan
from room_terrain import MAX_NATIVE_STEPS, MAX_PARTS, mask_digest, residual_parts

STRATEGY = 'bounded-original-dig/1'
POLICY = 'dfmcp.complete-residual-capacity/1'


def assess(plan: RoomPlan, costs: TileCosts, work: Callable[[], None]) -> dict:
    """Assess every residual target without changing the original room geometry.

    This internal boundary receives the actual compiler and parsed cost-map
    outputs, not imported evidence. Only explicit part-count excess is an
    infeasible candidate. Invalid data, interruption and exhausted work propagate
    to the enclosing caller, never becoming a no-layout or best-so-far result.
    """
    work()
    require(type(plan) is RoomPlan and type(costs) is TileCosts,
            'complete compiled room plan and parsed cost domain required')
    body = plan.json()
    targets = costs.target_mask(body, work)
    remaining_bits = targets & costs.charged
    x0, y0, z0 = costs.origin
    width = costs.size[0]
    remaining = set()
    for index in range(len(costs.cells)):
        work()
        if remaining_bits & (1 << index):
            remaining.add((x0 + index % width, y0 + index // width, z0))
    mask = frozenset(remaining)
    parts, native_parts = residual_parts(body, mask, work)
    fits = len(parts) <= MAX_PARTS and len(native_parts) <= MAX_NATIVE_STEPS
    # The empty mask is a valid no-work result, not a replacement dig blueprint.
    # Only nonempty, complete results can be exported by the existing handoff.
    if fits and mask:
        require(len(canonical({'schema': 'dfmcp.excavation-blueprint/1', 'parts': parts})) <= 16384,
                'complete residual blueprint exceeds existing byte allowance')
    result = {'policy': POLICY, 'remaining_tiles': len(mask),
              'remaining_mask_digest': mask_digest(mask),
              'blueprint_parts': len(parts), 'normal_mining_rectangles': len(native_parts),
              'maximum_blueprint_parts': MAX_PARTS, 'maximum_native_steps': MAX_NATIVE_STEPS,
              'within_profile_limits': fits, 'empty_residual': not mask,
              'partition_policy': 'smaller_global_or_original_part_width_first_cover',
              'minimum_parts_proven': False, 'terrain_observed': False,
              'native_excavation_eligibility_proven': False,
              'mutation_authority_granted': False}
    work()
    return result
