#include "../common/work_order_creation.h"
#include <cstdlib>
#include <memory>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/World.h"
#include "df/global_objects.h"
#include "df/item_type.h"
#include "df/job_type.h"
#include "df/manager_order.h"
#include "df/workquota_frequency_type.h"
#include "df/world.h"
#include "DfmcpWorkOrdersV1_10.pb.h"

using namespace DFHack;
namespace wire = dfmcp::work_orders::v1_10;
namespace wo = dfmcp_work_orders;
DFHACK_PLUGIN("dfmcp_work_orders_v1_10");
namespace {
wo::Engine engine(static_cast<std::uint64_t>(wo::Clock::now().time_since_epoch().count()) | 1);
bool enabled(const char *key) {
    const char *value = std::getenv(key); return value && std::string_view(value) == "1";
}
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(10); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in, out, 3);
    wo::require(in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty());
    wo::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    wo::require(in->protocol_major() == 1 && in->protocol_minor() == 10, 2);
    wo::require(enabled("DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10"), 1);
    const char *configured = std::getenv("DFMCP_WORK_ORDERS_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &given = in->bearer_token();
    wo::require(expected.size() >= 32 && expected.size() <= 256 && given.size() >= 32 && given.size() <= 256, 1);
    std::size_t difference = expected.size() ^ given.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < given.size() ? given[i] : 0;
        difference |= a ^ b;
    }
    wo::require(difference == 0, 1); wo::require(engine.available(), 5);
    const auto &version = Core::getInstance().vinfo;
    const std::string df = version ? version->getVersion() : std::string();
    const char *native_version = Version::dfhack_version(); wo::require(native_version != nullptr, 5);
    const std::string dfhack(native_version);
    wo::require(wo::utf8(df, 128) && wo::utf8(dfhack, 128), 5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
void require_fortress() {
    wo::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world, 4);
}
wo::Observation read_orders() {
    require_fortress();
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick(); const auto site = World::GetCurrentSiteId();
    wo::require(year >= 0 && year <= UINT32_MAX && tick < 403200 && site >= 0 && site <= INT32_MAX, 5);
    const auto &queue = df::global::world->manager_orders;
    wo::require(queue.manager_order_next_id >= 0 && queue.all.size() <= wo::MAX_ORDERS, 5);
    wo::Observation out; out.tick = static_cast<std::uint64_t>(year) * 403200 + tick;
    out.site = static_cast<std::uint32_t>(site); out.folder = World::ReadWorldFolder();
    out.paused = World::ReadPauseState(); out.next_order = static_cast<std::uint32_t>(queue.manager_order_next_id);
    out.ids.reserve(queue.all.size());
    for (const auto *order : queue.all) {
        wo::require(order && order->id >= 0, 5); out.ids.push_back(static_cast<std::uint32_t>(order->id));
    }
    std::sort(out.ids.begin(), out.ids.end()); // Presentation order is not insertion authority.
    return out;
}
df::job_type native_recipe(wo::Recipe recipe) {
    switch (recipe) {
        case wo::Recipe::WoodenBed: return df::job_type::ConstructBed;
        case wo::Recipe::WoodenDoor: return df::job_type::ConstructDoor;
        case wo::Recipe::WoodenTable: return df::job_type::ConstructTable;
        case wo::Recipe::WoodenChair: return df::job_type::ConstructThrone;
    }
    throw wo::Failure(3);
}
void configure(df::manager_order &order, std::uint32_t id, wo::Spec spec) {
    spec.validate(); wo::require(id < INT32_MAX, 5);
    order.id = static_cast<std::int32_t>(id); order.job_type = native_recipe(spec.recipe);
    order.item_type = df::item_type::NONE; order.item_subtype = -1;
    order.reaction_name.clear(); order.mat_type = -1; order.mat_index = -1;
    order.specflag.encrust_flags.whole = 0; order.specdata.hist_figure_id = -1;
    order.material_category.whole = 0; order.material_category.bits.wood = true;
    order.art_spec.id = -1; order.art_spec.subid = -1;
    order.amount_left = static_cast<std::int16_t>(spec.amount); order.amount_total = static_cast<std::int16_t>(spec.amount);
    order.status.whole = 0; order.frequency = df::workquota_frequency_type::OneTime;
    order.finished_year = -1; order.finished_year_tick = -1;
    order.workshop_id = -1; order.max_workshops = 1;
    wo::require(order.item_conditions.empty() && order.order_conditions.empty() && !order.items, 5);
}
// Own allocations until insertion. reserve() may throw but occurs before either
// semantic queue write; push_back of a pointer then cannot allocate. Do not force
// validation/activation, edit existing orders, unpause, or enqueue native jobs.
void create_order(std::uint32_t id, wo::Spec spec) {
    require_fortress(); wo::require(World::ReadPauseState(), 4);
    auto &queue = df::global::world->manager_orders;
    wo::require(queue.manager_order_next_id >= 0 && static_cast<std::uint32_t>(queue.manager_order_next_id) == id
        && id < INT32_MAX && queue.all.size() < wo::MAX_ORDERS, 6);
    auto order = std::make_unique<df::manager_order>(); configure(*order, id, spec);
    queue.all.reserve(queue.all.size() + 1);
    queue.all.push_back(order.get());
    ++queue.manager_order_next_id;
    (void)order.release();
}
std::string verify_order(std::uint32_t id, wo::Spec spec) {
    require_fortress();
    const auto &queue = df::global::world->manager_orders;
    wo::require(queue.all.size() <= wo::MAX_ORDERS, 5);
    const df::manager_order *found = nullptr;
    for (const auto *order : queue.all) {
        wo::require(order, 5);
        if (order->id >= 0 && static_cast<std::uint32_t>(order->id) == id) {
            wo::require(!found, 5); found = order;
        }
    }
    wo::require(found, 5); df::manager_order expected; configure(expected, id, spec);
    const auto &a = *found; const auto &b = expected;
    // Full fixed-template readback, not a count-only or type-only success check.
    wo::require(a.id == b.id && a.job_type == b.job_type && a.item_type == b.item_type && a.item_subtype == b.item_subtype
        && a.reaction_name == b.reaction_name && a.mat_type == b.mat_type && a.mat_index == b.mat_index
        && a.specflag.encrust_flags.whole == b.specflag.encrust_flags.whole && a.specdata.hist_figure_id == b.specdata.hist_figure_id
        && a.material_category.whole == b.material_category.whole
        && a.art_spec.type == b.art_spec.type && a.art_spec.id == b.art_spec.id && a.art_spec.subid == b.art_spec.subid
        && a.amount_left == b.amount_left && a.amount_total == b.amount_total && a.status.whole == b.status.whole
        && a.frequency == b.frequency && a.finished_year == b.finished_year && a.finished_year_tick == b.finished_year_tick
        && a.workshop_id == b.workshop_id && a.max_workshops == b.max_workshops
        && a.item_conditions.empty() && a.order_conditions.empty() && !a.items, 5);
    return wo::configuration(id, spec);
}
unsigned shape(const wire::Request *in) {
    return (in->has_idempotency_key() ? 1 : 0) | (in->has_recipe() ? 2 : 0)
        | (in->has_amount() ? 4 : 0) | (in->has_expected_witness() ? 8 : 0)
        | (in->has_plan_digest() ? 16 : 0) | (in->has_prepare_token() ? 32 : 0);
}
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out, Body body) {
    try {
        authorize(in, out); body(); out->set_failure_code(0); out->set_accepted(true); return CR_OK;
    } catch (const wo::Failure &e) {
        try { empty_reply(in, out, e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(in, out, 5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    }
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { wo::require(shape(in) == 0); });
}
command_result ReadOrders(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { wo::require(shape(in) == 0); out->set_observation(engine.inspect(read_orders).encode()); });
}
command_result PrepareOrder(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] {
        wo::require(shape(in) == 31); wo::require(enabled("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION"), 1);
        wo::require(in->recipe() >= 1 && in->recipe() <= 4);
        const wo::Spec spec{static_cast<wo::Recipe>(in->recipe()), in->amount()};
        const auto result = engine.prepare(in->idempotency_key(), spec, in->expected_witness(), in->plan_digest(), wo::Clock::now(), read_orders);
        out->set_effect_record(result.first->encode()); out->set_replayed(result.second);
    });
}
command_result CommitOrder(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] {
        wo::require(shape(in) == 49); wo::require(enabled("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION"), 1);
        const auto &record = engine.commit(in->idempotency_key(), in->plan_digest(), in->prepare_token(), wo::Clock::now(),
            read_orders, create_order, verify_order); out->set_effect_record(record.encode());
    });
}
command_result QueryOrder(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] {
        wo::require(shape(in) == 17);
        if (const auto *record = engine.query(in->idempotency_key(), in->plan_digest())) out->set_effect_record(record->encode());
    });
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { engine.reset(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_MAP_LOADED || event == SC_MAP_UNLOADED || event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED) engine.reset();
    else if (event == SC_PAUSED || event == SC_UNPAUSED) engine.interrupt();
    return CR_OK;
}
// Zero flags: DFHack's own suspended RPC dispatch owns every game access.
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0); service->addFunction("ReadOrders", ReadOrders, 0);
    service->addFunction("PrepareOrder", PrepareOrder, 0); service->addFunction("CommitOrder", CommitOrder, 0);
    service->addFunction("QueryOrder", QueryOrder, 0); return service;
}
