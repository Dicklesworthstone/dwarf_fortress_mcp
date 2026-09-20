#pragma once
#include "../common/order_run_wire.h"
#include <set>
#include "Core.h"
#include "modules/World.h"
#include "df/global_objects.h"
#include "df/item_type.h"
#include "df/job_type.h"
#include "df/manager_order.h"
#include "df/workquota_frequency_type.h"
#include "df/world.h"

namespace dfmcp_order_run_native {
namespace run = dfmcp_order_run;
// Same complete finite-wood recognition semantics as progress/1.12; dynamic
// status/left are values, not a promise of approval, production or eligibility.
inline std::uint8_t recipe(const df::manager_order &a) {
    std::uint8_t code = 0;
    switch (a.job_type) {
        case df::job_type::ConstructBed: code = 1; break;
        case df::job_type::ConstructDoor: code = 2; break;
        case df::job_type::ConstructTable: code = 3; break;
        case df::job_type::ConstructThrone: code = 4; break;
        default: return 0;
    }
    df::manager_order defaults;
    auto wood = defaults.material_category; wood.whole = 0; wood.bits.wood = true;
    const bool matches = a.item_type == df::item_type::NONE && a.item_subtype == -1
        && a.reaction_name.empty() && a.mat_type == -1 && a.mat_index == -1
        && a.specflag.encrust_flags.whole == 0 && a.specdata.hist_figure_id == -1
        && a.material_category.whole == wood.whole && a.art_spec.type == defaults.art_spec.type
        && a.art_spec.id == -1 && a.art_spec.subid == -1
        && a.amount_total >= 1 && a.amount_total <= 100 && a.amount_left >= 0 && a.amount_left <= a.amount_total
        && (a.status.whole & ~3u) == 0 && a.frequency == df::workquota_frequency_type::OneTime
        && a.workshop_id == -1 && a.max_workshops == 1
        && a.item_conditions.empty() && a.order_conditions.empty() && !a.items;
    return matches ? code : 0;
}
inline run::Identity identity(std::uint64_t generation) {
    using namespace DFHack;
    run::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world, 4);
    const auto site = World::GetCurrentSiteId();
    run::require(site >= 0 && site <= INT32_MAX, 5);
    run::Identity value{generation, static_cast<std::uint32_t>(site), World::ReadWorldFolder()};
    value.validate(); run::require(run::utf8(value.folder, 512), 5); return value;
}
inline run::Capture capture(std::uint64_t generation, std::uint32_t id) {
    using namespace DFHack;
    run::require(id <= INT32_MAX); run::Capture out; out.identity = identity(generation);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = static_cast<std::int64_t>(World::ReadCurrentTick());
    run::require(year >= 0 && year <= UINT32_MAX && tick >= 0 && tick < 403200, 5);
    out.clock = {generation, 0, static_cast<std::uint64_t>(year) * 403200 + static_cast<std::uint64_t>(tick),
        true, true, World::ReadPauseState()};
    const auto &queue = df::global::world->manager_orders;
    run::require(queue.manager_order_next_id >= 0 && queue.all.size() <= 4096, 5);
    out.id = id; out.next_order = static_cast<std::uint32_t>(queue.manager_order_next_id);
    std::set<std::int32_t> seen; const df::manager_order *target = nullptr;
    for (const auto *order : queue.all) {
        run::require(order && order->id >= 0 && order->id < queue.manager_order_next_id
            && seen.insert(order->id).second, 5);
        if (static_cast<std::uint32_t>(order->id) == id) target = order;
    }
    if (target) {
        out.present = true; out.recipe = recipe(*target); out.total = target->amount_total;
        out.left = target->amount_left; out.status = target->status.whole;
    }
    out.validate(); return out;
}
inline void set_pause(std::uint64_t generation, const run::Identity &expected, bool paused) {
    // Independent of target reads: corrupt queue/clock data must not disable a
    // same-fortress safety pause or enable a replacement-fortress setter.
    run::require(identity(generation) == expected, 4);
    DFHack::World::SetPauseState(paused);
}
} // namespace dfmcp_order_run_native
