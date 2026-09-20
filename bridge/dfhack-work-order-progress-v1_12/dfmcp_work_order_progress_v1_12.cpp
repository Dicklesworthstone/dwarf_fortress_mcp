#include "../common/work_order_progress.h"
#include <chrono>
#include <cstdlib>
#include <map>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "MiscUtils.h"
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
#include "DfmcpWorkOrderProgressV1_12.pb.h"

using namespace DFHack;
namespace wire = dfmcp::work_order_progress::v1_12;
namespace wp = dfmcp_work_order_progress;
DFHACK_PLUGIN("dfmcp_work_order_progress_v1_12");
namespace {
wp::Sequence sequence(static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count()) | 1);
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(12); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in, out, 3);
    wp::require(in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty());
    wp::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    wp::require(in->protocol_major() == 1 && in->protocol_minor() == 12, 2);
    const char *opt_in = std::getenv("DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12");
    wp::require(opt_in && std::string_view(opt_in) == "1", 1);
    const char *configured = std::getenv("DFMCP_WORK_ORDER_PROGRESS_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &given = in->bearer_token();
    wp::require(expected.size() >= 32 && expected.size() <= 256 && given.size() >= 32 && given.size() <= 256, 1);
    std::size_t difference = expected.size() ^ given.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < given.size() ? given[i] : 0;
        difference |= a ^ b;
    }
    wp::require(difference == 0, 1); wp::require(sequence.available(), 5);
    const auto &version = Core::getInstance().vinfo;
    const std::string df = version ? version->getVersion() : std::string();
    const char *v = Version::dfhack_version(); wp::require(v != nullptr, 5);
    const std::string dfhack(v); wp::require(wp::utf8(df, 128) && wp::utf8(dfhack, 128), 5);
    out->set_bridge_generation(sequence.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
std::uint8_t template_recipe(const df::manager_order &a) {
    std::uint8_t recipe = 0;
    switch (a.job_type) {
        case df::job_type::ConstructBed: recipe = 1; break;
        case df::job_type::ConstructDoor: recipe = 2; break;
        case df::job_type::ConstructTable: recipe = 3; break;
        case df::job_type::ConstructThrone: recipe = 4; break;
        default: return 0;
    }
    df::manager_order defaults;
    auto wood = defaults.material_category; wood.whole = 0; wood.bits.wood = true;
    // Dynamic status, remaining count and next-check time deliberately are NOT
    // identity fields. Unknown status bits cannot be recognized as this template.
    const bool matches = a.item_type == df::item_type::NONE && a.item_subtype == -1
        && a.reaction_name.empty() && a.mat_type == -1 && a.mat_index == -1
        && a.specflag.encrust_flags.whole == 0 && a.specdata.hist_figure_id == -1
        && a.material_category.whole == wood.whole && a.art_spec.type == defaults.art_spec.type
        && a.art_spec.id == -1 && a.art_spec.subid == -1
        && a.amount_total >= 1 && a.amount_total <= 100 && a.amount_left >= 0 && a.amount_left <= a.amount_total
        && (a.status.whole & ~3u) == 0 && a.frequency == df::workquota_frequency_type::OneTime
        && a.workshop_id == -1 && a.max_workshops == 1
        && a.item_conditions.empty() && a.order_conditions.empty() && !a.items;
    return matches ? recipe : 0;
}
wp::Row row(const df::manager_order &a) {
    wp::require(a.id >= 0 && a.item_conditions.size() <= wp::MAX_QUEUE && a.order_conditions.size() <= wp::MAX_QUEUE, 5);
    wp::Row out; out.id = static_cast<std::uint32_t>(a.id); out.present = true;
    out.job_type = static_cast<std::int32_t>(a.job_type); out.recipe = template_recipe(a);
    out.left = a.amount_left; out.total = a.amount_total; out.status = a.status.whole;
    out.frequency = static_cast<std::int32_t>(a.frequency); out.workshop = a.workshop_id;
    out.max_workshops = a.max_workshops; out.next_check_year = a.finished_year; out.next_check_tick = a.finished_year_tick;
    out.item_conditions = static_cast<std::uint32_t>(a.item_conditions.size());
    out.order_conditions = static_cast<std::uint32_t>(a.order_conditions.size());
    out.type_key = ENUM_KEY_STR(job_type, a.job_type); out.reaction = a.reaction_name;
    return out;
}
wp::Observation capture(const std::vector<std::uint32_t> &ids) {
    wp::targets(ids);
    wp::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world, 4);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = static_cast<std::int64_t>(World::ReadCurrentTick()); const auto site = World::GetCurrentSiteId();
    wp::require(year >= 0 && year <= UINT32_MAX && tick >= 0 && tick < 403200 && site >= 0 && site <= INT32_MAX, 5);
    const auto &queue = df::global::world->manager_orders;
    wp::require(queue.manager_order_next_id >= 0 && queue.all.size() <= wp::MAX_QUEUE, 5);
    // Scan every queue entry BEFORE declaring any selected ID absent. Pointers
    // are scoped to this suspended call, not cached or returned on the wire.
    std::map<std::uint32_t, const df::manager_order *> index;
    for (const auto *order : queue.all) {
        wp::require(order && order->id >= 0 && order->id < queue.manager_order_next_id, 5);
        wp::require(index.emplace(static_cast<std::uint32_t>(order->id), order).second, 5);
    }
    wp::Observation out; sequence.stamp(out);
    out.tick = static_cast<std::uint64_t>(year) * 403200 + static_cast<std::uint64_t>(tick); out.site = static_cast<std::uint32_t>(site);
    out.next_order = static_cast<std::uint32_t>(queue.manager_order_next_id);
    out.queue_count = static_cast<std::uint32_t>(queue.all.size()); out.paused = World::ReadPauseState();
    out.folder = World::ReadWorldFolder(); out.rows.reserve(ids.size());
    for (auto id : ids) {
        const auto found = index.find(id);
        if (found == index.end()) { wp::Row absent; absent.id = id; out.rows.push_back(absent); }
        else out.rows.push_back(row(*found->second));
    }
    return out;
}
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out, Body body) {
    try {
        authorize(in, out); body(); out->set_failure_code(0); out->set_accepted(true); return CR_OK;
    } catch (const wp::Failure &e) {
        try { empty_reply(in, out, e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(in, out, 5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    }
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { wp::require(in->native_order_ids_size() == 0); });
}
command_result ReadObservation(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] {
        wp::require(in->native_order_ids_size() > 0 && in->native_order_ids_size() <= static_cast<int>(wp::MAX_TARGETS));
        const std::vector<std::uint32_t> ids(in->native_order_ids().begin(), in->native_order_ids().end());
        out->set_observation(capture(ids).encode());
    });
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { sequence.reset(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_MAP_LOADED || event == SC_MAP_UNLOADED || event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED) sequence.reset();
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0); service->addFunction("ReadObservation", ReadObservation, 0);
    return service;
}
