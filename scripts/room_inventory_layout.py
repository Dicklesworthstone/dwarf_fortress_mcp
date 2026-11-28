"""Site a complete room project against one released operations/1.4 inventory."""
from __future__ import annotations

from construction_receipt import Manifest, decode_operations
import furniture_inventory as inventory
from furniture_allocation import allocate
from furniture_plan import canonical, require
import room_layout as layout
from room_furniture_handoff import RoomFurnitureHandoff
from room_provisioning import RoomPlan

SCHEMA = 'dfmcp.inventory-room-layout/1'
MAX_REPORT = 160 * 1024


def prepare(raw: bytes, guard: layout.Guard):
    request, profiles, work = layout.prepare_request(raw, guard)
    for area in request['areas']:
        for constraint in area['item_constraints'].values():
            require(not constraint.get('max_vertical_distance'),
                    'cross-level item movement requires the observed route allocator')
    return request, profiles, work


def evaluate(prepared, capture: bytes, manifest: Manifest, address: str) -> dict:
    request, profiles, work = prepared
    observed = decode_operations(capture, work)
    require((observed.folder, observed.site) == (request['world_folder'], request['site']),
            'inventory is not from the requested fortress')
    inventory.endpoint(address)
    manifest.encode()
    candidates, counts = inventory.eligible_items(observed, request['excluded_items'], work)
    tested = shortages = 0

    def has_stock(plan: RoomPlan) -> bool:
        nonlocal tested, shortages
        allocation = allocate(plan.request(), candidates, work)
        tested += 1
        if allocation['status'] == 'shortage':
            shortages += 1
            return False
        require(allocation['status'] == 'allocated', 'unrecognized allocation outcome')
        return True

    result = layout._search(request, profiles, work, has_stock)
    handoff = None
    if result['status'] == 'layout_found':
        plan = RoomPlan.decode(canonical(result['room_plan']), work)
        slots = plan.request()
        selection = allocate(slots, candidates, work)
        require(selection['status'] == 'allocated', 'selected room lost its complete inventory allocation')
        allocation = inventory.bind_allocation(slots, selection, observed, address, manifest, work)
        handoff = RoomFurnitureHandoff(plan, allocation, checkpoint=work).json()

    source = {**inventory.source_reference(observed), 'endpoint': address,
              'native_generation': manifest.generation, 'df_version': manifest.df_version,
              'dfhack_version': manifest.dfhack_version}
    report = {'schema': SCHEMA, 'request': request, 'source': source,
              'projection': inventory.projection_summary(observed, counts),
              'status': result['status'], 'search': result['search'],
              'complete_layouts_checked_for_inventory': tested,
              'layouts_rejected_for_inventory_shortage': shortages,
              'room_handoff': handoff, 'terrain_observed': False, 'items_reserved': False,
              'game_effect_performed': False, 'production_admitted': False}
    if 'optimization' in result:
        report['optimization'] = dict(result['optimization'])
        report['optimization']['scope'] += '_with_complete_captured_inventory_assignment'
    if 'residual_partition' in result:
        report['residual_partition'] = result['residual_partition']
    work()
    report['search']['work_units'] = work.used
    require(len(canonical(report)) <= MAX_REPORT, 'complete inventory layout report exceeds its byte bound')
    return report


def acquire(raw: bytes, authority: inventory.Authority, budget: inventory.Budget, emit: str) -> bytes:
    authority.guard()

    def guard():
        budget.work()
        # During CPU-only search, poll read authority at bounded intervals.
        # The client checks it at I/O boundaries and again before publication.
        if budget.work_steps % 128 == 0:
            authority.guard()
    prepared = prepare(raw, guard)
    with inventory.InventoryClient(authority, budget) as client:
        manifest, capture = client.capture_once()
    report = evaluate(prepared, capture, manifest, f'{authority.address[0]}:{authority.address[1]}')
    report.update(native_capture_established=True, capture_release_verified=True)
    if emit == 'room-handoff':
        require(report['room_handoff'] is not None, 'a complete room allocation is required for export')
        result = canonical(report['room_handoff'])
    else:
        require(emit == 'report', 'unsupported inventory layout export')
        result = canonical(report)
    guard()
    authority.guard()
    return result
