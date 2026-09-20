#include "native_capture.h"
#include <atomic>
#include <cstdlib>
#include <memory>
#include <string_view>
#include <vector>
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "DfmcpOrderRunV1_14.pb.h"

using namespace DFHack;
namespace wire = dfmcp::order_run::v1_14;
namespace run = dfmcp_order_run;
DFHACK_PLUGIN("dfmcp_order_run_v1_14");
// No disable hook: a disconnected client must not disable the native stop owner.
namespace {
run::Engine engine;
std::uint64_t generation = static_cast<std::uint64_t>(run::Clock::now().time_since_epoch().count()) | 1;
enum class Gate : unsigned char { Idle, Busy, Closing };
std::atomic<Gate> gate{Gate::Idle};
std::atomic<bool> drain_requested{false};
run::Capture read_native(std::uint32_t id) { return dfmcp_order_run_native::capture(generation, id); }
void write_native(const run::Identity &expected, bool paused) { dfmcp_order_run_native::set_pause(generation, expected, paused); }
void release_idle_gate() {
    if (!engine.active()) { auto expected = Gate::Busy; (void)gate.compare_exchange_strong(expected, Gate::Idle); }
}
void service() {
    if (drain_requested.load()) (void)engine.shutdown(run::Clock::now(), read_native, write_native);
    else engine.service(run::Clock::now(), read_native, write_native);
    release_idle_gate();
}
void initialize_reply(const wire::Request &in, wire::Reply &out) {
    out.Clear(); out.set_accepted(false); out.set_failure_code(3);
    out.set_client_nonce(in.client_nonce().size() >= 16 && in.client_nonce().size() <= 64 ? in.client_nonce() : std::string());
    out.set_protocol_major(1); out.set_protocol_minor(14); out.set_bridge_generation(0);
    out.set_df_version(""); out.set_dfhack_version("");
}
void authenticate(const wire::Request &in, wire::Reply &out) {
    run::require(in.IsInitialized() && in.ByteSizeLong() <= 2048
        && in.GetReflection()->GetUnknownFields(in).field_count() == 0);
    run::require(in.client_nonce().size() >= 16 && in.client_nonce().size() <= 64);
    run::require(in.protocol_major() == 1 && in.protocol_minor() == 14, 2);
    const char *opt = std::getenv("DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14");
    run::require(opt && std::string_view(opt) == "1" && !std::getenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL"), 1);
    const char *configured = std::getenv("DFMCP_ORDER_RUN_TOKEN");
    const std::string_view expected = configured ? configured : ""; const auto &presented = in.bearer_token();
    run::require(expected.size() >= 32 && expected.size() <= 256 && presented.size() >= 32 && presented.size() <= 256, 1);
    std::size_t diff = expected.size() ^ presented.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < presented.size() ? presented[i] : 0;
        diff |= a ^ b;
    }
    run::require(diff == 0, 1);
    const auto &info = Core::getInstance().vinfo;
    const std::string df = info ? info->getVersion() : std::string();
    const char *version = Version::dfhack_version(); run::require(version != nullptr, 5);
    const std::string dfhack(version);
    run::require(generation && generation != UINT64_MAX && run::utf8(df, 128) && run::utf8(dfhack, 128), 5);
    out.set_bridge_generation(generation); out.set_df_version(df); out.set_dfhack_version(dfhack);
}
enum class Operation { Handshake, Observe, Prepare, Commit, Query, Cancel };
void check_shape(const wire::Request &in, Operation op) {
    const bool prepare = op == Operation::Prepare, observe = op == Operation::Observe;
    const bool keyed = op != Operation::Handshake && !observe;
    const bool token = op == Operation::Commit || op == Operation::Cancel;
    run::require(in.has_idempotency_key() == keyed && in.has_plan_digest() == keyed
        && in.has_game_ticks() == prepare && in.has_wall_ms() == prepare && in.has_expected_capture() == prepare
        && in.has_prepare_token() == token && in.has_native_order_id() == (observe || prepare)
        && in.has_predicate() == prepare && in.has_threshold() == prepare
        && in.has_stable_samples() == prepare && in.has_interval_ticks() == prepare);
    if (keyed) { run::clock::valid_key(in.idempotency_key()); run::require(in.plan_digest().size() == 32); }
    if (token) run::require(in.prepare_token().size() == 16);
    if (observe || prepare) run::require(in.native_order_id() <= INT32_MAX);
    if (prepare) run::require(in.predicate() >= 1 && in.predicate() <= 3);
}
void execute(const wire::Request &in, wire::Reply &out, Operation op) {
    check_shape(in, op);
    if (op == Operation::Observe) out.set_observation(run::encode_capture(engine.observe(in.native_order_id(), read_native)));
    else if (op == Operation::Prepare) {
        run::require(!drain_requested.load() && gate.load() != Gate::Closing, 8);
        const run::clock::Spec limits{in.game_ticks(), in.wall_ms()};
        const run::Goal goal{static_cast<run::Predicate>(in.predicate()), in.threshold(), in.stable_samples(), in.interval_ticks()};
        const auto before = run::decode_capture(in.expected_capture());
        run::require(before.id == in.native_order_id() && in.plan_digest() == run::plan_digest(limits, goal, before), 7);
        const auto &record = engine.prepare(in.idempotency_key(), in.plan_digest(), limits, goal, before, run::Clock::now(), read_native);
        out.set_effect_record(run::encode_record(record));
    } else if (op == Operation::Commit || op == Operation::Query || op == Operation::Cancel) {
        const auto *known = engine.query(in.idempotency_key(), in.plan_digest());
        if (op != Operation::Query) {
            run::require(known && in.prepare_token() == run::token_for(known->run->key, known->run->plan), 7);
            if (op == Operation::Cancel) known = &engine.cancel(in.idempotency_key(), in.plan_digest(), run::Clock::now(), read_native, write_native);
            else if (known->run->phase == run::clock::Phase::Prepared) {
                auto expected = Gate::Idle;
                run::require(!drain_requested.load() && gate.compare_exchange_strong(expected, Gate::Busy), 8);
                run::require(!drain_requested.load(), 8);
                known = &engine.commit(in.idempotency_key(), in.plan_digest(), run::Clock::now(), read_native, write_native);
            }
        }
        if (known) out.set_effect_record(run::encode_record(*known));
    }
    out.set_owner_active(engine.active()); out.set_retained_records(static_cast<std::uint32_t>(engine.size()));
}
command_result dispatch(const wire::Request *in, wire::Reply *out, Operation op) {
    CoreSuspender suspend;
    try {
        initialize_reply(*in, *out); authenticate(*in, *out); execute(*in, *out, op);
        release_idle_gate(); out->set_accepted(true); out->set_failure_code(0); return CR_OK;
    } catch (const run::clock::Failure &e) {
        release_idle_gate(); try { initialize_reply(*in, *out); out->set_failure_code(e.code); } catch (...) { return CR_FAILURE; }
    } catch (...) {
        // Effect ownership survives even when response allocation fails.
        release_idle_gate(); try { initialize_reply(*in, *out); out->set_failure_code(5); } catch (...) { return CR_FAILURE; }
    }
    return CR_OK;
}
#define ORDER_RUN_RPC(name, operation) \
    command_result name(color_ostream &, const wire::Request *in, wire::Reply *out) { return dispatch(in, out, Operation::operation); }
ORDER_RUN_RPC(Handshake, Handshake)
ORDER_RUN_RPC(ObserveRun, Observe)
ORDER_RUN_RPC(PrepareRun, Prepare)
ORDER_RUN_RPC(CommitRun, Commit)
ORDER_RUN_RPC(QueryRun, Query)
ORDER_RUN_RPC(CancelRun, Cancel)
#undef ORDER_RUN_RPC
}
DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &) { return CR_OK; }
DFhackCExport command_result plugin_onupdate(color_ostream &) { CoreSuspender suspend; service(); return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event) {
    if (event == SC_BEGIN_UNLOAD) {
        // Never acquire the core lock from the plugin-access-lock unload hook.
        drain_requested.store(true); auto expected = Gate::Idle;
        return gate.compare_exchange_strong(expected, Gate::Closing) || expected == Gate::Closing ? CR_OK : CR_FAILURE;
    }
    if (event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED || event == SC_MAP_LOADED || event == SC_MAP_UNLOADED) {
        CoreSuspender suspend; if (generation != UINT64_MAX) ++generation;
        engine.source_changed(); release_idle_gate();
    }
    return CR_OK;
}
DFhackCExport command_result plugin_shutdown(color_ostream &) { return engine.active() ? CR_FAILURE : CR_OK; }
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto out = std::make_unique<RPCService>();
    out->addFunction("Handshake", Handshake, 0); out->addFunction("ObserveRun", ObserveRun, 0);
    out->addFunction("PrepareRun", PrepareRun, 0); out->addFunction("CommitRun", CommitRun, 0);
    out->addFunction("QueryRun", QueryRun, 0); out->addFunction("CancelRun", CancelRun, 0); return out.release();
}
