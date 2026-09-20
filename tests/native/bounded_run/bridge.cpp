// Compile the actual plugin translation unit against explicitly named SDK and
// protobuf API doubles. This is not a real DFHack SDK or protobuf ABI test.
#include <iostream>
#include <thread>
#include <functional>
#include "bridge/dfhack-run-v1_13/dfmcp_run_v1_13.cpp"
namespace {
int assertions = 0;
void check(bool value) { ++assertions; if (!value) throw std::runtime_error("bridge assertion " + std::to_string(assertions)); }
color_ostream output;
using Handler = command_result (*)(color_ostream &, const wire::Request *, wire::Reply *);
wire::Request base() {
    wire::Request request; request.set_bearer_token(std::string(32, 's'));
    request.set_client_nonce(std::string(16, 'n')); request.set_protocol_major(1); request.set_protocol_minor(13);
    return request;
}
wire::Reply invoke(Handler handler, const wire::Request &request, unsigned expected = 0) {
    wire::Reply reply; check(handler(output, &request, &reply) == CR_OK);
    check(reply.IsInitialized()); check(reply.failure_code() == expected); check(reply.accepted() == (expected == 0));
    if (expected) check(!reply.has_effect_record() && !reply.has_observation());
    return reply;
}
wire::Request prepare(const std::string &key, unsigned ticks = 10, unsigned wall = 60000) {
    const auto observed = invoke(ObserveRun, base());
    auto request = base(); request.set_idempotency_key(key); request.set_game_ticks(ticks); request.set_wall_ms(wall);
    request.set_expected_observation(observed.observation());
    request.set_plan_digest(run::digest_plan({ticks, wall}, run::decode_snapshot(observed.observation())));
    const auto reply = invoke(PrepareRun, request); check(reply.has_effect_record());
    check(reply.effect_record() == run::encode_record(*engine.query(key, request.plan_digest())));
    return request;
}
wire::Request identity(const wire::Request &prepared, bool with_token = true) {
    auto request = base(); request.set_idempotency_key(prepared.idempotency_key()); request.set_plan_digest(prepared.plan_digest());
    if (with_token) request.set_prepare_token(run::token_for(request.idempotency_key(), request.plan_digest()));
    return request;
}
const run::Record &record(const wire::Request &prepared) { return *engine.query(prepared.idempotency_key(), prepared.plan_digest()); }
void update() { check(plugin_onupdate(output) == CR_OK); check(fake::depth == 0); }
std::string hex(const std::string &bytes) {
    constexpr char digits[] = "0123456789abcdef"; std::string out;
    for (unsigned char c : bytes) { out.push_back(digits[c >> 4]); out.push_back(digits[c & 15]); }
    return out;
}
void authorization_and_shapes() {
    unsetenv("DFMCP_ALLOW_UNADMITTED_RUN_V1_13"); invoke(Handshake, base(), 1);
    setenv("DFMCP_ALLOW_UNADMITTED_RUN_V1_13", "1", 1); setenv("DFMCP_RUN_TOKEN", std::string(32, 's').c_str(), 1);
    setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "1.0", 1); invoke(Handshake, base(), 1); unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    auto request = base(); request.set_bearer_token(std::string(32, 'x')); invoke(Handshake, request, 1);
    request = base(); request.set_client_nonce("bad"); invoke(Handshake, request, 3);
    request = base(); request.set_protocol_minor(7); invoke(Handshake, request, 2);
    request = base(); request.set_idempotency_key("extra"); invoke(Handshake, request, 3);
    request = base(); request.unknown_count = 1; invoke(Handshake, request, 3);
    request = base(); request.clear_protocol_major(); invoke(Handshake, request, 3);
    request = base(); request.set_bearer_token(std::string(3000, 's')); invoke(Handshake, request, 3);
    const auto hello = invoke(Handshake, base()); check(!hello.has_observation() && !hello.has_effect_record());
    check(hello.bridge_generation() == generation && hello.df_version() == "fake-df");
    std::unique_ptr<RPCService> rpc(plugin_rpcconnect(output));
    check(rpc->names == std::vector<std::string>({"Handshake", "ObserveRun", "PrepareRun", "CommitRun", "QueryRun", "CancelRun"}));
    check(fake::unpauses == 0 && fake::pauses == 0);
}
void bounds_and_seals() {
    const auto prepared = prepare("sealed");
    invoke(PrepareRun, prepared); // Identical preparation is stable.
    auto altered = prepared; altered.set_game_ticks(11); invoke(PrepareRun, altered, 7);
    altered = prepared; altered.set_game_ticks(0); invoke(PrepareRun, altered, 3);
    altered = prepared; altered.set_wall_ms(60001); invoke(PrepareRun, altered, 3);
    altered = prepared; altered.set_expected_observation(std::string(36, 'x')); invoke(PrepareRun, altered, 3);
    altered = identity(prepared); altered.set_prepare_token(std::string(16, 'x')); invoke(CommitRun, altered, 7);
    altered = identity(prepared); altered.set_wall_ms(100); invoke(CommitRun, altered, 3);
    altered = identity(prepared, false); altered.set_idempotency_key("absent");
    check(!invoke(QueryRun, altered).has_effect_record());
    invoke(CancelRun, identity(prepared)); invoke(CommitRun, identity(prepared));
    check(record(prepared).phase == run::Phase::Refused && fake::unpauses == 0);
    const run::Snapshot snapshot{41, 3, 806500, true, true, true};
    const auto bytes = run::encode_snapshot(snapshot); check(run::decode_snapshot(bytes) == snapshot);
    run::Record golden; golden.key = "golden"; golden.spec = {10, 1000}; golden.before = snapshot;
    golden.plan = run::digest_plan(golden.spec, snapshot);
    std::cout << "snapshot=" << hex(bytes) << "\nplan=" << hex(golden.plan)
        << "\nrecord=" << hex(run::encode_record(golden)) << '\n';
    for (std::size_t i = 32; i < 35; ++i) {
        auto invalid = bytes; invalid[i] = 2;
        bool rejected = false; try { (void)run::decode_snapshot(invalid); } catch (const run::Failure &) { rejected = true; }
        check(rejected);
    }
}
void callback_after_disconnect_and_retries() {
    auto prepared = prepare("disconnected");
    const auto competing = prepare("competing");
    {
        std::unique_ptr<RPCService> connection(plugin_rpcconnect(output));
        invoke(CommitRun, identity(prepared));
    } // Client's native RPC service is gone; global safety ownership must remain.
    check(engine.active() && !fake::paused && gate.load() == Gate::Busy);
    const auto unpauses = fake::unpauses;
    invoke(CommitRun, identity(prepared)); check(fake::unpauses == unpauses);
    invoke(CommitRun, identity(competing), 8);
    fake::tick += 9; update(); check(engine.active());
    fake::tick += 1; update(); check(!engine.active() && fake::paused && gate.load() == Gate::Idle);
    check(record(prepared).reason == run::Reason::TickLimit && record(prepared).pause_verified);
    const auto result = invoke(QueryRun, identity(prepared, false)); check(result.has_effect_record());
    invoke(CommitRun, identity(prepared)); check(fake::unpauses == unpauses);
    invoke(CommitRun, identity(competing)); check(record(competing).phase == run::Phase::Refused);
}
void cancellation_and_ambiguous_delivery() {
    auto prepared = prepare("cancel-running"); invoke(CommitRun, identity(prepared));
    fake::fail_pause = true; invoke(CancelRun, identity(prepared));
    check(engine.active() && record(prepared).phase == run::Phase::Stopping && !record(prepared).pause_verified);
    update(); check(engine.active()); fake::fail_pause = false; update();
    check(record(prepared).pause_verified && record(prepared).reason == run::Reason::Cancelled);
    prepared = prepare("reply-lost"); fake_proto::fail_effect = true;
    invoke(CommitRun, identity(prepared), 5);
    check(engine.active() && !fake::paused && record(prepared).unpause_attempted);
    const auto unpauses = fake::unpauses; invoke(CommitRun, identity(prepared)); check(fake::unpauses == unpauses);
    fake::tick += 12; update(); check(record(prepared).pause_verified);
    check(record(prepared).observed_tick - record(prepared).before.tick == 12);
    prepared = prepare("setter-ambiguous"); fake::fail_after_unpause = true;
    invoke(CommitRun, identity(prepared)); fake::fail_after_unpause = false;
    check(record(prepared).reason == run::Reason::NativeFailure && record(prepared).pause_verified);
    prepared = prepare("no-op-unpause"); fake::ignore_unpause = true;
    invoke(CommitRun, identity(prepared)); fake::ignore_unpause = false;
    check(!engine.active() && record(prepared).reason == run::Reason::ExternalPause);
}
void clock_source_and_wall() {
    auto prepared = prepare("wall-only", 1200, 1); invoke(CommitRun, identity(prepared));
    std::this_thread::sleep_for(std::chrono::milliseconds(3)); update();
    check(record(prepared).reason == run::Reason::WallLimit && record(prepared).pause_verified);
    prepared = prepare("clock-invalid"); invoke(CommitRun, identity(prepared)); fake::year = -1; update();
    check(record(prepared).pause_verified && !record(prepared).tick_known); fake::year = 2;
    prepared = prepare("new-fort"); invoke(CommitRun, identity(prepared)); const auto pauses = fake::pauses;
    fake::map_loaded = false; check(plugin_onstatechange(output, SC_MAP_UNLOADED) == CR_OK);
    fake::map_loaded = true; fake::paused = false; check(plugin_onstatechange(output, SC_MAP_LOADED) == CR_OK); update();
    check(record(prepared).phase == run::Phase::SourceLost && !record(prepared).pause_verified && fake::pauses == pauses);
    invoke(CommitRun, identity(prepared)); check(!fake::paused); fake::paused = true;
}
void unload_veto_and_racing_commit() {
    const auto prepared = prepare("unload-race");
    fake::on_unpause = [] {
        check(gate.load() == Gate::Busy);
        check(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_FAILURE);
    };
    invoke(CommitRun, identity(prepared)); fake::on_unpause = {};
    fake::fail_pause = true; update();
    check(engine.active() && record(prepared).reason == run::Reason::Shutdown);
    check(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_FAILURE);
    fake::fail_pause = false; update(); check(record(prepared).pause_verified && !engine.active());
    invoke(PrepareRun, prepared, 8);
    check(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_OK);
    check(gate.load() == Gate::Closing && plugin_shutdown(output) == CR_OK);
}
} // namespace
int main() {
    generation = 41;
    std::vector<PluginCommand> commands; check(plugin_init(output, commands) == CR_OK);
    authorization_and_shapes(); bounds_and_seals(); callback_after_disconnect_and_retries();
    cancellation_and_ambiguous_delivery(); clock_source_and_wall(); unload_veto_and_racing_commit();
    std::cout << "bridge_assertions=" << assertions << '\n';
}
