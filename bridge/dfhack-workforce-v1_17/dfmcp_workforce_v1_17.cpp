#include "../common/workforce_control.h"
#include <cstdlib>
#include <iterator>
#include <memory>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "MiscUtils.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/Units.h"
#include "modules/World.h"
#include "df/gamest.h"
#include "df/global_objects.h"
#include "df/plotinfost.h"
#include "df/unit.h"
#include "df/unit_labor.h"
#include "df/work_detail.h"
#include "df/work_detail_mode.h"
#include "df/world.h"
#include "DfmcpWorkforceV1_17.pb.h"

using namespace DFHack;
namespace wf = dfmcp_workforce;
namespace wire = dfmcp::workforce::v1_17;
DFHACK_PLUGIN("dfmcp_workforce_v1_17");
namespace {
wf::Engine engine(static_cast<std::uint64_t>(wf::Clock::now().time_since_epoch().count()) | 1);
void control_enabled() {
    const char *v = std::getenv("DFMCP_WORKFORCE_ALLOW_LABOR");
    wf::require(v && std::string_view(v) == "1", 1);
}
void source_available() {
    wf::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode() && df::global::world && df::global::plotinfo && df::global::game, 4);
}
std::map<std::uint32_t, df::unit *> selected_units(const std::vector<std::uint32_t> &ids) {
    wf::ids_ok(ids, wf::MAX_UNITS); source_available();
    const auto &roster = df::global::world->units.active;
    wf::require(roster.size() <= wf::MAX_MEMBERS, 5);
    std::set<std::int32_t> seen; std::map<std::uint32_t, df::unit *> result;
    for (auto *u : roster) {
        wf::require(u && u->id >= 0 && seen.insert(u->id).second, 5);
        if (std::binary_search(ids.begin(), ids.end(), static_cast<std::uint32_t>(u->id))) {
            // Never expose arbitrary invader/visitor labor data through a guessed ID.
            wf::require(Units::isActive(u) && Units::isCitizen(u, true) && u->hist_figure_id >= 0, 4);
            result.emplace(static_cast<std::uint32_t>(u->id), u);
        }
    }
    wf::require(result.size() == ids.size(), 4); return result;
}
wf::Capture read_native(const std::vector<std::uint32_t> &ids) {
    const auto selected = selected_units(ids);
    const auto &details = df::global::plotinfo->labor_info.work_details;
    wf::require(details.size() <= wf::MAX_DETAILS, 5);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = static_cast<std::int64_t>(World::ReadCurrentTick());
    const auto site = World::GetCurrentSiteId();
    wf::require(year >= 0 && year <= UINT32_MAX && tick >= 0 && tick < 403200 && site >= 0, 5);
    wf::Capture out; out.tick = static_cast<std::uint64_t>(year) * 403200 + static_cast<std::uint64_t>(tick);
    out.site = static_cast<std::uint32_t>(site); out.folder = World::ReadWorldFolder();
    out.paused = World::ReadPauseState();
    out.automatic = !df::global::game->external_flag.bits.automatic_professions_disabled;
    const auto n = std::size(selected.begin()->second->status.labors);
    wf::require(n > 0 && n <= wf::MAX_LABORS, 5);
    for (std::size_t i = 0; i < n; ++i)
        out.labor_keys.push_back(ENUM_KEY_STR(unit_labor, static_cast<df::unit_labor>(i)));
    std::size_t count = 0;
    for (const auto *d : details) {
        wf::require(d && std::size(d->allowed_labors) == n && d->name.size() <= 256, 5);
        count += d->assigned_units.size(); wf::require(count <= wf::MAX_MEMBERS, 5);
        wf::Detail value; value.name = DF2UTF(d->name); value.flags = d->flags.whole;
        value.selected_only = d->flags.bits.mode == df::work_detail_mode::OnlySelectedDoesThis;
        for (auto id : d->assigned_units) { wf::require(id >= 0, 5); value.members.push_back(static_cast<std::uint32_t>(id)); }
        for (std::size_t i = 0; i < n; ++i) value.labors.push_back(d->allowed_labors[i] ? 1 : 0);
        out.details.push_back(std::move(value));
    }
    for (const auto &[id, u] : selected) {
        wf::Unit value; value.id = id; value.historical_id = static_cast<std::uint32_t>(u->hist_figure_id);
        value.eligible = Units::isCitizen(u) && Units::isAdult(u);
        for (std::size_t i = 0; i < n; ++i) value.labors.push_back(u->status.labors[i] ? 1 : 0);
        out.units.push_back(std::move(value));
    }
    return out;
}
void write_native(const wf::Capture &before, wf::Spec spec,
    const std::vector<std::uint32_t> &members, const std::vector<std::uint32_t> &changed) {
    control_enabled(); source_available();
    wf::require(engine.generation() == before.generation, 6);
    auto current = read_native(before.ids()); current.generation = before.generation; current.sequence = before.sequence;
    wf::require(current.encode() == before.encode(), 6);
    const auto units = selected_units(before.ids());
    auto *detail = df::global::plotinfo->labor_info.work_details.at(spec.detail);
    // Prepare the entire replacement vector and all citizen references BEFORE
    // the first write. Units uses the supported Bay12 recomputation entry point.
    std::vector<std::int32_t> replacement; replacement.reserve(members.size());
    for (auto id : members) { wf::require(id <= INT32_MAX, 5); replacement.push_back(static_cast<std::int32_t>(id)); }
    std::vector<df::unit *> targets; targets.reserve(changed.size());
    for (auto id : changed) targets.push_back(units.at(id));
    detail->assigned_units.swap(replacement);
    for (auto *u : targets) Units::setAutomaticProfessions(u);
    // Exceptions/partial recomputation remain Unknown in the already-published
    // engine record. There is deliberately no rollback or second setter attempt.
}
void empty_reply(const wire::Request &in, wire::Reply &out, std::uint32_t code) {
    out.Clear(); out.set_accepted(false); out.set_failure_code(code);
    out.set_client_nonce(in.client_nonce().size() >= 16 && in.client_nonce().size() <= 64 ? in.client_nonce() : std::string());
    out.set_protocol_major(1); out.set_protocol_minor(17); out.set_bridge_generation(0);
    out.set_df_version(""); out.set_dfhack_version("");
}
void authenticate(const wire::Request &in, wire::Reply &out) {
    wf::require(in.IsInitialized() && in.ByteSizeLong() <= 2048
        && in.GetReflection()->GetUnknownFields(in).field_count() == 0);
    wf::require(in.client_nonce().size() >= 16 && in.client_nonce().size() <= 64);
    wf::require(in.protocol_major() == 1 && in.protocol_minor() == 17, 2);
    const char *opt = std::getenv("DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17");
    wf::require(opt && std::string_view(opt) == "1" && !std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL"), 1);
    const char *configured = std::getenv("DFMCP_WORKFORCE_TOKEN");
    const std::string_view secret = configured ? configured : "";
    const auto &given = in.bearer_token();
    wf::require(secret.size() >= 32 && secret.size() <= 256 && given.size() >= 32 && given.size() <= 256, 1);
    std::size_t difference = secret.size() ^ given.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < secret.size() ? secret[i] : 0, b = i < given.size() ? given[i] : 0;
        difference |= a ^ b;
    }
    wf::require(difference == 0, 1);
    const auto &info = Core::getInstance().vinfo; const char *version = Version::dfhack_version();
    wf::require(info && version && engine.generation() && engine.generation() != UINT64_MAX, 5);
    const std::string df = info->getVersion(), dfhack = version;
    wf::require(wf::utf8(df, 128) && wf::utf8(dfhack, 128), 5);
    out.set_bridge_generation(engine.generation()); out.set_df_version(df); out.set_dfhack_version(dfhack);
}
enum class Operation { Handshake, Observe, Prepare, Commit, Query, Cancel };
void execute(const wire::Request &in, wire::Reply &out, Operation op) {
    const bool plain = op == Operation::Handshake || op == Operation::Observe;
    const bool prepare = op == Operation::Prepare;
    const bool token = op == Operation::Commit || op == Operation::Cancel;
    const bool selected = op == Operation::Observe || prepare;
    wf::require(in.has_idempotency_key() == !plain && in.has_plan_digest() == !plain
        && in.has_detail_index() == prepare && in.has_assigned() == prepare
        && in.has_expected_witness() == prepare && in.has_prepare_token() == token
        && (in.unit_ids_size() > 0) == selected && in.unit_ids_size() <= static_cast<int>(wf::MAX_UNITS));
    const std::vector<std::uint32_t> ids(in.unit_ids().begin(), in.unit_ids().end());
    if (!plain) { wf::key_ok(in.idempotency_key()); wf::require(in.plan_digest().size() == 32); }
    if (token) wf::require(in.prepare_token().size() == 16);
    if (prepare || token) control_enabled();
    const wf::Record *record = nullptr;
    if (op == Operation::Observe) out.set_observation(engine.observe(ids, read_native).encode());
    else if (prepare) record = &engine.prepare(in.idempotency_key(), {in.detail_index(), in.assigned()},
        ids, in.expected_witness(), in.plan_digest(), wf::Clock::now(), read_native);
    else if (op == Operation::Commit) record = &engine.commit(in.idempotency_key(), in.plan_digest(),
        in.prepare_token(), wf::Clock::now(), read_native, write_native);
    else if (op == Operation::Query) record = engine.query(in.idempotency_key(), in.plan_digest());
    else if (op == Operation::Cancel) record = &engine.cancel(in.idempotency_key(), in.plan_digest(), in.prepare_token());
    if (record) out.set_effect_record(record->encode());
    out.set_unresolved(engine.unresolved()); out.set_retained_records(static_cast<std::uint32_t>(engine.size()));
}
command_result dispatch(const wire::Request *in, wire::Reply *out, Operation op) {
    CoreSuspender suspend;
    try {
        empty_reply(*in, *out, 3); authenticate(*in, *out); execute(*in, *out, op);
        out->set_accepted(true); out->set_failure_code(0); return CR_OK;
    } catch (const wf::Failure &e) {
        try { empty_reply(*in, *out, e.code); } catch (...) { return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(*in, *out, 5); } catch (...) { return CR_FAILURE; }
    }
    return CR_OK;
}
#define WF_RPC(name, op) \
command_result name(color_ostream &, const wire::Request *in, wire::Reply *out) { return dispatch(in, out, Operation::op); }
WF_RPC(Handshake, Handshake)
WF_RPC(ObserveWorkforce, Observe)
WF_RPC(PrepareAssignment, Prepare)
WF_RPC(CommitAssignment, Commit)
WF_RPC(QueryAssignment, Query)
WF_RPC(CancelAssignment, Cancel)
#undef WF_RPC
}
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED || event == SC_MAP_LOADED || event == SC_MAP_UNLOADED) {
        CoreSuspender suspend; engine.reset();
    }
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto service = std::make_unique<RPCService>();
    service->addFunction("Handshake", Handshake, 0); service->addFunction("ObserveWorkforce", ObserveWorkforce, 0);
    service->addFunction("PrepareAssignment", PrepareAssignment, 0); service->addFunction("CommitAssignment", CommitAssignment, 0);
    service->addFunction("QueryAssignment", QueryAssignment, 0); service->addFunction("CancelAssignment", CancelAssignment, 0);
    return service.release();
}
