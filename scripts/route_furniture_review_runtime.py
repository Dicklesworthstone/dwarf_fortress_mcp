"""Query-only acquisition for original route-furniture reviews.

Reuse the existing map/inventory connection, full-roster decoder and candidate
policy. No new wire protocol, credential, placement path or source issuer.
"""
from __future__ import annotations

import hashlib

from furniture_allocation import Request
from furniture_plan import canonical, integer, require
import route_furniture_allocation as routes
import route_furniture_inventory as inventory
import route_furniture_review as review_model
import route_furniture_sequence as sequencing

# Fixed work reservation for creating/validating/serializing the bounded artifact.
# It is never borrowed from the already reserved route solver allowance.
EXPORT_WORK = 100_000


def acquire(request, region, authority, budget, original=None):
    require(type(request) is Request and type(region) is inventory.Region
            and type(authority) is inventory.Authority, 'closed review acquisition inputs required')
    request.__post_init__()
    region.__post_init__()
    authority.guard()
    budget.remaining()
    if original is not None:
        require(type(original) is review_model.Source, 'invalid original source')
        require(inventory.wire.endpoint(original.endpoint) == authority.address,
                'operator endpoint differs from original review')
        require((original.origin, original.size) == (region.origin, region.size),
                'review cannot replace its selected region')
    with inventory.Client(authority, region, budget) as client:
        if original is not None:
            expected = (original.map_generation, original.inventory_generation,
                        original.df_version, original.dfhack_version)
            actual = (client.manifests['map'].generation, client.manifests['operations'].generation,
                      *client.manifests['map'].software)
            require(actual == expected, 'current handshake differs from original source')
        map_manifest, map_raw, operations_manifest, operations_raw = client.capture_once()
    require(client.closed, 'review source must close before projection')
    terrain = routes.decode_terrain(map_raw, budget.work)
    require((terrain.origin, terrain.size) == (region.origin, region.size), 'map substituted region')
    observed = inventory.decode_operations(operations_raw, budget.work)
    require((terrain.folder, terrain.site) == (observed.folder, observed.site) == (request.folder, request.site),
            'review captures belong to another fortress')
    require(terrain.paused and observed.paused and terrain.tick == observed.tick,
            'review captures do not share one paused tick')
    require(map_manifest.software == operations_manifest.software, 'review source software differs')
    source = review_model.Source(f'{authority.address[0]}:{authority.address[1]}',
        *map_manifest.software, map_manifest.generation, operations_manifest.generation,
        terrain.digest, observed.digest, terrain.tick, terrain.dimensions,
        terrain.origin, terrain.size, observed.horizons)
    if original is not None:
        original.permits(source)
    candidates, counts = inventory.candidates_from(observed, request, budget.work)
    native_types = {}
    for item in candidates:
        budget.work()
        native_types[item.id] = observed.items[item.id].native_type
    authority.guard()
    budget.remaining()
    return map_raw, source, candidates, native_types, counts


def packet(result=None, request=None, source=None, *, operation='check', review=None, guard=review_model.idle):
    value = inventory.packet(result, request, source)
    value['schema'] = 'dfmcp.route-furniture-review-inventory-result/1'
    value['review'] = None if review is None else review.json(guard)
    value['review_digest'] = (review.digest if review is not None else
                              result.get('review_digest') if result is not None else None)
    turn = value['agent_turn']
    turn['operation'] = 'furniture.route_review.' + operation
    turn['phase'] = 'propose' if operation == 'create' else 'inspect'
    turn['coverage']['original_selection_review'] = result is not None and operation == 'check'
    turn['coverage']['placement_gate_enforced'] = False
    turn['uncertainty'].append(
        'Retain the complete review and its expected digest separately. Rechecking original selections neither enforces a native placement gate nor resolves any outstanding effect.')
    if result is not None and 'delivery_sequence' in result:
        sequence = result['delivery_sequence']
        turn['coverage']['delivery_order_model'] = sequence['model']
        turn['coverage']['delivery_order_checked'] = sequence['candidate_model_sequence_proven']
        turn['uncertainty'].append(
            'Delivery order models earlier target tiles as blocked. It preserves selected items and original constraints, but is not native hauling or a guarantee of future routes. Fresh placement review remains required.')
    if result is None:
        value['error'] = {'code': 'ROUTE_FURNITURE_REVIEW_REFUSED',
            'detail': 'Original review, source, authority, evidence or budget refused. No current review established; no placement was attempted.'}
    return value


def finish(result, request, source, counts, authority, budget, *, operation, review=None):
    reference = {'policy': inventory.ACQUISITION_POLICY, **source.json(),
                 'candidate_policy': inventory.CANDIDATE_POLICY, 'counts': counts,
                 'complete_roster_decoded': True, 'pure_result_digest': result['decision_digest'],
                 'atomic_cross_profile_snapshot_proven': False,
                 'native_acquisition_independently_attested': False}
    reference['evidence_digest'] = hashlib.sha256(
        b'dfmcp-route-review-acquisition/1\0' + canonical(reference)).hexdigest()
    value = packet(result, request, reference, operation=operation, review=review, guard=budget.work)
    output = inventory.bounded_output(value)
    budget.remaining()
    authority.guard()
    return output


def create(request: Request, region: inventory.Region, authority: inventory.Authority,
           budget: inventory.wire.Budget, *, max_route_steps=routes.MAX_ROUTE_STEPS,
           sequence_deliveries: bool = False) -> bytes:
    require(type(sequence_deliveries) is bool, 'delivery sequencing requires explicit Boolean opt-in')
    integer(max_route_steps, 0, routes.MAX_ROUTE_STEPS)
    require(budget.work_steps > EXPORT_WORK, 'review export work must be reserved before native contact')
    raw, source, candidates, types, counts = acquire(request, region, authority, budget)
    allowance = min(routes.MAX_WORK, budget.work_steps - EXPORT_WORK)
    require(allowance > 0, 'insufficient work for allocation and complete review export')
    budget.work_steps -= allowance

    def guard():
        budget.remaining()
        authority.guard()

    if sequence_deliveries:
        # Use the shared compiler once, and keep its strengthened request with
        # the allocation through review creation and final publication. Never
        # apply this to a retained review or an already-started batch.
        request, result = sequencing.allocate_sequenced_routes(
            request, candidates, raw, guard, max_route_steps=max_route_steps, maximum_work=allowance)
    else:
        result = routes.allocate_routes(request, candidates, raw, guard,
                                        max_route_steps=max_route_steps, maximum_work=allowance)
    review = None
    if result['status'] == 'allocated':
        selected_types = {row['item']: types[row['item']] for row in result['assignments']}
        review = review_model.create(request, result, source, selected_types, budget.work)
    guard()
    return finish(result, request, source, counts, authority, budget, operation='create', review=review)


def check(review: review_model.Review, authority: inventory.Authority,
          budget: inventory.wire.Budget) -> bytes:
    require(type(review) is review_model.Review, 'complete original review required')
    _, request, original, _ = review.unpack(budget.work)
    region = inventory.Region(original.origin, original.size)
    raw, source, candidates, types, counts = acquire(request, region, authority, budget, original)
    allowance = min(routes.MAX_WORK, budget.work_steps)
    require(allowance > 0, 'no original selection review work remains')
    budget.work_steps -= allowance

    def guard():
        budget.remaining()
        authority.guard()

    result = review_model.revalidate(review, candidates, types, raw, source, guard, maximum_work=allowance)
    return finish(result, request, source, counts, authority, budget, operation='check')
