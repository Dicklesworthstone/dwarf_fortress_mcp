#include "../common/job_suspension.h"
#include <cstdlib>
#include <string_view>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "MiscUtils.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/Job.h"
#include "modules/World.h"
#include "df/building.h"
#include "df/building_type.h"
#include "df/global_objects.h"
#include "df/job.h"
#include "df/unit.h"
#include "DfmcpJobControlV1_9.pb.h"

using namespace DFHack;
namespace wire = dfmcp::job_control::v1_9;
namespace js = dfmcp_job_suspension;
DFHACK_PLUGIN("dfmcp_job_control_v1_9");
namespace {
js::Engine engine(static_cast<std::uint64_t>(js::Clock::now().time_since_epoch().count()) | 1);

void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(9); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in, out, 3);
    js::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    js::require(in->protocol_major() == 1 && in->protocol_minor() == 9, 2);
    const char *configured = std::getenv("DFMCP_JOB_CONTROL_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &given = in->bearer_token();
    js::require(expected.size() >= 32 && expected.size() <= 256 && given.size() >= 32 && given.size() <= 256, 1);
    std::size_t difference = expected.size() ^ given.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < given.size() ? given[i] : 0;
        difference |= a ^ b;
    }
    js::require(difference == 0, 1); js::require(engine.available(), 5);
    const auto &version = Core::getInstance().vinfo;
    const std::string df = version ? version->getVersion() : std::string();
    const char *native_version = Version::dfhack_version();
    js::require(native_version != nullptr, 5); const std::string dfhack(native_version);
    js::require(js::utf8(df, 128) && js::utf8(dfhack, 128), 5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
// flags=0 registration gives DFHack-owned CoreSuspender serialization. There is
// no onUpdate work, detached task or pointer retained outside that suspension.
js::Observation read_job(std::uint32_t id) {
    js::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded() && World::isFortressMode(), 4);
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick(); const auto site = World::GetCurrentSiteId();
    js::require(year >= 0 && year <= UINT32_MAX && tick < 403200 && site >= 0 && site <= INT32_MAX, 5);
    js::require(df::global::job_next_id && *df::global::job_next_id >= 0 && id <= INT32_MAX, 5);
    auto *job = Job::getJob(static_cast<int>(id)); js::require(job != nullptr, 4);
    js::require(job->id >= 0 && static_cast<std::uint32_t>(job->id) == id
        && job->general_refs.size() <= 4096 && job->items.size() <= 65536 && job->job_items.elements.size() <= 4096, 5);
    for (const auto *ref : job->general_refs) js::require(ref != nullptr, 5);
    auto *worker = Job::getWorker(job); auto *holder = Job::getHolder(job);
    js::require((!worker || worker->id >= 0) && (!holder || holder->id >= 0), 5);
    js::Observation out; out.job = id; out.next_job = *df::global::job_next_id; out.site = static_cast<std::uint32_t>(site);
    out.tick = static_cast<std::uint64_t>(year) * 403200 + tick; out.folder = World::ReadWorldFolder();
    out.type = static_cast<std::int32_t>(job->job_type); out.type_key = ENUM_KEY_STR(job_type, job->job_type);
    out.reaction = job->reaction_name; out.suspended = job->flags.bits.suspend; out.repeating = job->flags.bits.repeat;
    out.paused = World::ReadPauseState(); out.worker = worker ? worker->id : -1; out.holder = holder ? holder->id : -1;
    out.x = job->pos.x; out.y = job->pos.y; out.z = job->pos.z; out.timer = job->completion_timer;
    out.attachments = static_cast<std::uint32_t>(job->items.size()); out.filters = static_cast<std::uint32_t>(job->job_items.elements.size());
    out.supported = Job::isSupportedJob(job);
    if (holder) {
        const auto kind = holder->getType(); out.holder_type = static_cast<std::int32_t>(kind);
        const auto stage = holder->getBuildStage(), maximum = holder->getMaxBuildStage();
        js::require(stage >= 0 && maximum >= stage, 5);
        out.holder_complete = stage == maximum;
        out.production_holder = kind == df::building_type::Workshop || kind == df::building_type::Furnace;
    }
    return out;
}
void set_suspended(std::uint32_t id, bool desired) {
    auto *job = Job::getJob(static_cast<int>(id)); js::require(job != nullptr, 5);
    // read_job/Engine just checked paused, idle, supported and completed holder.
    // Do not detach a worker, delete a job, change repeat, or unpause simulation.
    job->flags.bits.suspend = desired;
}
unsigned shape(const wire::Request *in) {
    return (in->has_idempotency_key() ? 1 : 0) | (in->has_native_job_id() ? 2 : 0)
        | (in->has_suspended() ? 4 : 0) | (in->has_expected_witness() ? 8 : 0)
        | (in->has_plan_digest() ? 16 : 0) | (in->has_prepare_token() ? 32 : 0);
}
template<class Body> command_result guarded(const wire::Request *in, wire::Reply *out, Body body) {
    try {
        authorize(in, out); body(); out->set_failure_code(0); out->set_accepted(true); return CR_OK;
    } catch (const js::Failure &e) {
        try { empty_reply(in, out, e.code); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    } catch (...) {
        try { empty_reply(in, out, 5); return CR_OK; } catch (...) { out->Clear(); return CR_FAILURE; }
    }
}
command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { js::require(shape(in) == 0); });
}
command_result ReadJob(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { js::require(shape(in) == 2);
        out->set_observation(engine.inspect(in->native_job_id(), read_job).encode()); });
}
command_result PrepareSuspension(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { js::require(shape(in) == 31);
        const auto result = engine.prepare(in->idempotency_key(), in->native_job_id(), in->suspended(),
            in->expected_witness(), in->plan_digest(), js::Clock::now(), read_job);
        out->set_effect_record(result.first->encode()); out->set_replayed(result.second); });
}
command_result CommitSuspension(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { js::require(shape(in) == 49);
        const auto &record = engine.commit(in->idempotency_key(), in->plan_digest(), in->prepare_token(),
            js::Clock::now(), read_job, set_suspended); out->set_effect_record(record.encode()); });
}
command_result QuerySuspension(color_ostream &, const wire::Request *in, wire::Reply *out) {
    return guarded(in, out, [&] { js::require(shape(in) == 17);
        if (const auto *record = engine.query(in->idempotency_key(), in->plan_digest())) out->set_effect_record(record->encode()); });
}
} // namespace
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_shutdown(color_ostream &) { engine.reset(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_MAP_LOADED || event == SC_MAP_UNLOADED || event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED) engine.reset();
    else if (event == SC_PAUSED || event == SC_UNPAUSED) engine.interrupt();
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0); service->addFunction("ReadJob", ReadJob, 0);
    service->addFunction("PrepareSuspension", PrepareSuspension, 0); service->addFunction("CommitSuspension", CommitSuspension, 0);
    service->addFunction("QuerySuspension", QuerySuspension, 0); return service;
}
