#include "../common/bounded_run_wire.h"
#include <atomic>
#include <cstdlib>
#include <memory>
#include <string_view>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/World.h"
#include "DfmcpRunV1_13.pb.h"

using namespace DFHack;
namespace wire = dfmcp::run::v1_13;
namespace run = dfmcp_bounded_run;
DFHACK_PLUGIN("dfmcp_run_v1_13");
// Intentionally no plugin_enable hook: the safety callback remains enabled for
// the entire loaded lifetime, not for the lifetime of an RPC connection.
namespace {
run::Engine engine;
std::uint64_t generation = static_cast<std::uint64_t>(run::Clock::now().time_since_epoch().count()) | 1;
// SC_BEGIN_UNLOAD is invoked under DFHack's plugin-access lock. Never acquire a
// CoreSuspender there: publish a drain request and veto until the update callback
// proves quiescence. Claim Busy BEFORE any commit can cross the unpause boundary.
enum class Gate : unsigned char { Idle, Busy, Closing };
std::atomic<Gate> gate{Gate::Idle};
std::atomic<bool> drain_requested{false};

void release_idle_gate() {
    if (!engine.active()) {
        auto expected = Gate::Busy;
        (void)gate.compare_exchange_strong(expected, Gate::Idle);
    }
}
run::Snapshot read_native() {
    run::Snapshot value;
    value.generation = generation;
    value.loaded = Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded()
        && World::isFortressMode();
    if (!value.loaded) return value;
    value.paused = World::ReadPauseState();
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto tick = World::ReadCurrentTick();
    value.clock_valid = year >= 0 && year <= static_cast<std::int64_t>(UINT32_MAX) && tick < 403200;
    if (value.clock_valid) value.tick = static_cast<std::uint64_t>(year) * 403200 + tick;
    return value;
}
void write_native(std::uint64_t expected_generation, bool paused) {
    // Independent of read_native(): a failing read must not accidentally allow
    // the safety setter to operate on a different local-map incarnation.
    run::require(expected_generation == generation && Core::getInstance().isWorldLoaded()
        && Core::getInstance().isMapLoaded() && World::isFortressMode(), 4);
    World::SetPauseState(paused);
}
void service() {
    if (drain_requested.load())
        (void)engine.shutdown(run::Clock::now(), read_native, write_native);
    else
        engine.service(run::Clock::now(), read_native, write_native);
    release_idle_gate();
}
void initialize_reply(const wire::Request &in, wire::Reply &out) {
    out.Clear(); out.set_accepted(false); out.set_failure_code(3);
    out.set_client_nonce(in.client_nonce().size() >= 16 && in.client_nonce().size() <= 64
        ? in.client_nonce() : std::string());
    out.set_protocol_major(1); out.set_protocol_minor(13);
    out.set_bridge_generation(0); out.set_df_version(""); out.set_dfhack_version("");
}
void authenticate(const wire::Request &in, wire::Reply &out) {
    run::require(in.IsInitialized() && in.ByteSizeLong() <= 2048
        && in.GetReflection()->GetUnknownFields(in).field_count() == 0);
    run::require(in.client_nonce().size() >= 16 && in.client_nonce().size() <= 64);
    run::require(in.protocol_major() == 1 && in.protocol_minor() == 13, 2);
    const char *opt_in = std::getenv("DFMCP_ALLOW_UNADMITTED_RUN_V1_13");
    run::require(opt_in && std::string_view(opt_in) == "1"
        && !std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL"), 1);
    const char *configured = std::getenv("DFMCP_RUN_TOKEN");
    const std::string_view expected = configured ? std::string_view(configured) : std::string_view();
    const auto &presented = in.bearer_token();
    run::require(expected.size() >= 32 && expected.size() <= 256
        && presented.size() >= 32 && presented.size() <= 256, 1);
    std::size_t diff = expected.size() ^ presented.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0;
        const unsigned char b = i < presented.size() ? presented[i] : 0;
        diff |= a ^ b;
    }
    run::require(diff == 0, 1);
    const auto &info = Core::getInstance().vinfo;
    const std::string df = info ? info->getVersion() : std::string();
    const std::string dfhack = Version::dfhack_version();
    run::require(generation && generation != UINT64_MAX && !df.empty() && df.size() <= 128
        && !dfhack.empty() && dfhack.size() <= 128, 5);
    out.set_bridge_generation(generation); out.set_df_version(df); out.set_dfhack_version(dfhack);
}
enum class Operation { Handshake, Observe, Prepare, Commit, Query, Cancel };
void check_shape(const wire::Request &in, Operation op) {
    const bool plain = op == Operation::Handshake || op == Operation::Observe;
    const bool prepare = op == Operation::Prepare;
    const bool token = op == Operation::Commit || op == Operation::Cancel;
    run::require(in.has_idempotency_key() == !plain && in.has_plan_digest() == !plain
        && in.has_game_ticks() == prepare && in.has_wall_ms() == prepare
        && in.has_expected_observation() == prepare && in.has_prepare_token() == token);
    if (!plain) { run::valid_key(in.idempotency_key()); run::require(in.plan_digest().size() == 32); }
    if (token) run::require(in.prepare_token().size() == 16);
}
void execute(const wire::Request &in, wire::Reply &out, Operation op) {
    check_shape(in, op);
    if (op == Operation::Observe) out.set_observation(run::encode_snapshot(engine.observe(read_native)));
    else if (op == Operation::Prepare) {
        run::require(!drain_requested.load() && gate.load() != Gate::Closing, 8);
        const run::Spec spec{in.game_ticks(), in.wall_ms()};
        const auto before = run::decode_snapshot(in.expected_observation());
        run::require(in.plan_digest() == run::digest_plan(spec, before), 7);
        const auto &record = engine.prepare(in.idempotency_key(), in.plan_digest(), spec,
            before, run::Clock::now(), read_native);
        out.set_effect_record(run::encode_record(record));
    } else if (op == Operation::Commit || op == Operation::Query || op == Operation::Cancel) {
        const auto *known = engine.query(in.idempotency_key(), in.plan_digest());
        if (op != Operation::Query) {
            run::require(known && in.prepare_token() == run::token_for(known->key, known->plan), 7);
            if (op == Operation::Cancel) {
                known = &engine.cancel(in.idempotency_key(), in.plan_digest(), run::Clock::now(), read_native, write_native);
            } else if (known->phase == run::Phase::Prepared) {
                auto expected = Gate::Idle;
                run::require(!drain_requested.load() && gate.compare_exchange_strong(expected, Gate::Busy), 8);
                // A racing unload observes Busy and must veto, including if the
                // following check or any reply allocation fails.
                run::require(!drain_requested.load(), 8);
                known = &engine.commit(in.idempotency_key(), in.plan_digest(), run::Clock::now(), read_native, write_native);
            }
        }
        if (known) out.set_effect_record(run::encode_record(*known));
    }
    out.set_owner_active(engine.active()); out.set_retained_records(static_cast<std::uint32_t>(engine.size()));
}
command_result dispatch(const wire::Request *in, wire::Reply *out, Operation op) {
    // Explicit suspension also covers native callbacks in the executable test
    // harness; normal DFHack RPC flags=0 already hold the suspension.
    CoreSuspender suspend;
    try {
        initialize_reply(*in, *out); authenticate(*in, *out); execute(*in, *out, op);
        release_idle_gate(); out->set_accepted(true); out->set_failure_code(0); return CR_OK;
    } catch (const run::Failure &error) {
        release_idle_gate();
        try { initialize_reply(*in, *out); out->set_failure_code(error.code); }
        catch (...) { return CR_FAILURE; }
    } catch (...) {
        release_idle_gate();
        // A post-setter failure is NOT proof of non-application. Native clock
        // ownership remains published even if no response can be allocated.
        try { initialize_reply(*in, *out); out->set_failure_code(5); }
        catch (...) { return CR_FAILURE; }
    }
    return CR_OK;
}
#define RUN_RPC(name, operation) \
    command_result name(color_ostream &, const wire::Request *in, wire::Reply *out) { \
        return dispatch(in, out, Operation::operation); \
    }
RUN_RPC(Handshake, Handshake)
RUN_RPC(ObserveRun, Observe)
RUN_RPC(PrepareRun, Prepare)
RUN_RPC(CommitRun, Commit)
RUN_RPC(QueryRun, Query)
RUN_RPC(CancelRun, Cancel)
#undef RUN_RPC
} // namespace

DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_onupdate(color_ostream &) {
    CoreSuspender suspend;
    service(); return CR_OK;
}
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_BEGIN_UNLOAD) {
        drain_requested.store(true);
        auto expected = Gate::Idle;
        return gate.compare_exchange_strong(expected, Gate::Closing) || expected == Gate::Closing
            ? CR_OK : CR_FAILURE;
    }
    if (event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED
        || event == SC_MAP_LOADED || event == SC_MAP_UNLOADED) {
        CoreSuspender suspend;
        if (generation != UINT64_MAX) ++generation;
        engine.source_changed(); release_idle_gate();
    }
    return CR_OK;
}
DFhackCExport command_result plugin_shutdown(color_ostream &) {
    // The unload veto above is the safety boundary. Returning failure only here
    // would be too late: DFHack has already disabled onupdate at that stage.
    return engine.active() ? CR_FAILURE : CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto result = std::make_unique<RPCService>();
    result->addFunction("Handshake", Handshake, 0); result->addFunction("ObserveRun", ObserveRun, 0);
    result->addFunction("PrepareRun", PrepareRun, 0); result->addFunction("CommitRun", CommitRun, 0);
    result->addFunction("QueryRun", QueryRun, 0); result->addFunction("CancelRun", CancelRun, 0);
    return result.release();
}
