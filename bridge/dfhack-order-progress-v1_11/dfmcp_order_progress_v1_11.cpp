#include "../common/order_progress.h"
#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <vector>
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
#include "DfmcpOrderProgressV1_11.pb.h"

using namespace DFHack;
namespace wire = dfmcp::order_progress::v1_11;
namespace op = dfmcp_order_progress;
DFHACK_PLUGIN("dfmcp_order_progress_v1_11");
namespace {
op::Reader reader(static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count()) | 1);
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(11); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in,out,3);
    op::require(in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty());
    op::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    op::require(in->protocol_major() == 1 && in->protocol_minor() == 11,2);
    const char *enabled = std::getenv("DFMCP_ALLOW_UNADMITTED_ORDER_PROGRESS_V1_11");
    op::require(enabled && std::string_view(enabled) == "1",1);
    const char *configured = std::getenv("DFMCP_ORDER_PROGRESS_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &given = in->bearer_token();
    op::require(expected.size() >= 32 && expected.size() <= 256 && given.size() >= 32 && given.size() <= 256,1);
    std::size_t difference = expected.size() ^ given.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < given.size() ? given[i] : 0;
        difference |= a ^ b;
    }
    op::require(difference == 0,1); op::require(reader.available(),5);
    const auto &version = Core::getInstance().vinfo;
    const auto df = version ? version->getVersion() : std::string();
    const char *native = Version::dfhack_version(); op::require(native != nullptr,5);
    const std::string dfhack(native); op::require(op::utf8(df,128) && op::utf8(dfhack,128),5);
    out->set_bridge_generation(reader.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
// Recognize the complete finite template, not merely the job type. Dynamic
// counters, approval/activity and scheduling timestamps are not configuration.
std::uint8_t furniture_template(const df::manager_order &a) {
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
    if (a.item_type != df::item_type::NONE || a.item_subtype != -1 || !a.reaction_name.empty()
        || a.mat_type != -1 || a.mat_index != -1 || a.specflag.encrust_flags.whole != 0
        || a.specdata.hist_figure_id != -1 || a.material_category.whole != wood.whole
        || a.art_spec.type != defaults.art_spec.type || a.art_spec.id != -1 || a.art_spec.subid != -1
        || a.frequency != df::workquota_frequency_type::OneTime || a.workshop_id != -1 || a.max_workshops != 1
        || !a.item_conditions.empty() || !a.order_conditions.empty() || a.items
        || a.amount_total < 1 || a.amount_total > 100 || a.amount_left < 0 || a.amount_left > a.amount_total
        || (a.status.whole & ~3u)) return 0;
    return recipe;
}
op::Observation capture(std::uint32_t id) {
    op::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world,4);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick(); const auto site = World::GetCurrentSiteId();
    op::require(year >= 0 && year <= UINT32_MAX && tick < 403200 && site >= 0 && site <= INT32_MAX,5);
    const auto &queue = df::global::world->manager_orders;
    op::require(queue.manager_order_next_id >= 0 && queue.all.size() <= op::MAX_ORDERS,5);
    op::Observation out; out.order = id; out.next_order = static_cast<std::uint32_t>(queue.manager_order_next_id);
    out.site = static_cast<std::uint32_t>(site); out.tick = static_cast<std::uint64_t>(year) * 403200 + tick;
    out.paused = World::ReadPauseState(); out.folder = World::ReadWorldFolder();
    std::vector<std::uint32_t> ids; ids.reserve(queue.all.size());
    const df::manager_order *found = nullptr;
    // Full bounded scan must succeed even after finding the selected order.
    // This is what makes a missing selection an observed absence in this queue.
    for (const auto *order : queue.all) {
        op::require(order && order->id >= 0 && order->id < queue.manager_order_next_id,5);
        const auto native_id = static_cast<std::uint32_t>(order->id); ids.push_back(native_id);
        if (native_id == id) found = order;
    }
    std::sort(ids.begin(),ids.end());
    op::require(std::adjacent_find(ids.begin(),ids.end()) == ids.end(),5);
    if (found) {
        out.present = true; out.job_type = static_cast<std::int32_t>(found->job_type);
        op::require(found->amount_left >= 0 && found->amount_total >= 0,5);
        out.left = static_cast<std::uint32_t>(found->amount_left); out.total = static_cast<std::uint32_t>(found->amount_total);
        out.status = found->status.whole; out.frequency = static_cast<std::int32_t>(found->frequency);
        out.recipe = furniture_template(*found);
    }
    return out;
}
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out, Body body) {
    try { authorize(in,out); body(); out->set_failure_code(0); out->set_accepted(true); return CR_OK; }
    catch (const op::Failure &e) {
        try { empty_reply(in,out,e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(in,out,5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    }
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in,out,[&] { op::require(!in->has_native_order_id()); });
}
command_result ReadOrderProgress(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in,out,[&] { op::require(in->has_native_order_id());
        out->set_observation(reader.read(in->native_order_id(),capture).encode()); });
}
}
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { reader.reset(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_MAP_LOADED || event == SC_MAP_UNLOADED || event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED) reader.reset();
    return CR_OK;
}
// Zero flags: all game access stays inside DFHack-owned suspended RPC dispatch.
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service = new RPCService(); service->addFunction("Handshake",Handshake,0);
    service->addFunction("ReadOrderProgress",ReadOrderProgress,0); return service;
}
