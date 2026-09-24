#include "bridge/dfhack-excavation-run-v1_18/dfmcp_excavation_run_v1_18.cpp"
#include <iostream>
#include <sstream>
#include <stdexcept>

static unsigned assertions = 0, scenarios = 0, next_key = 0;
#define CHECK(expr) do { ++assertions; if (!(expr)) throw std::runtime_error(#expr); } while (false)
static color_ostream output;
static const std::string credential(32, 's');
using Rpc = command_result (*)(color_ostream &, const wire::Request *, wire::Reply *);
static wire::Request base() {
    wire::Request in; in.set_bearer_token(credential); in.set_client_nonce(std::string(32, 'n'));
    in.set_protocol_major(1); in.set_protocol_minor(18); return in;
}
static wire::Reply call(Rpc rpc, const wire::Request &in, bool accepted = true) {
    wire::Reply out; CHECK(rpc(output, &in, &out) == CR_OK); CHECK(out.IsInitialized());
    CHECK(out.accepted() == accepted); CHECK(out.ByteSizeLong() <= 4096);
    CHECK(fake::suspension == 0); return out;
}
static void setup() {
    CHECK(!engine.active());
    fake::loaded = fake::map_loaded = fake::fortress = fake::paused = true;
    fake::year = 0; fake::tick = 100; fake::site = 2; fake::folder = "region1";
    fake::sx = fake::sy = 64; fake::sz = 8;
    fake::fail_capture = fake::fail_pause = fake::noop_pause = fake::fail_unpause = fake::throw_reply = false;
    fake::setter_hook = {}; fake::terrain(false);
    gate.store(Gate::Idle); drain_requested.store(false);
    CHECK(plugin_onstatechange(output, SC_MAP_LOADED) == CR_OK);
    CHECK(setenv("DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18", "1", 1) == 0);
    CHECK(setenv("DFMCP_EXCAVATION_RUN_ALLOW_CLOCK", "1", 1) == 0);
    CHECK(setenv("DFMCP_EXCAVATION_RUN_TOKEN", credential.c_str(), 1) == 0);
    CHECK(unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL") == 0);
}
static wire::Request observation_request() {
    auto in = base(); in.set_x(15); in.set_y(15); in.set_z(2); in.set_width(2); in.set_height(2); return in;
}
struct Prepared {
    std::string key, plan;
    wire::Request request;
    const run::Record &record() const { return *engine.query(key, plan); }
    wire::Request keyed(bool token) const {
        auto in = base(); in.set_idempotency_key(key); in.set_plan_digest(plan);
        if (token) in.set_prepare_token(run::token_for(key, plan));
        return in;
    }
};
static Prepared preparation(bool submit = true) {
    const auto observed = call(ObserveRun, observation_request());
    const auto capture = run::decode_capture(observed.observation());
    const run::clock::Spec limits{100, 1000}; const run::Goal goal{2, 2, 1, 10};
    Prepared out; out.key = "excavate" + std::to_string(++next_key);
    out.plan = run::plan_digest(limits, goal, capture); out.request = observation_request();
    out.request.set_idempotency_key(out.key); out.request.set_plan_digest(out.plan);
    out.request.set_game_ticks(100); out.request.set_wall_ms(1000); out.request.set_expected_capture(observed.observation());
    out.request.set_stable_samples(2); out.request.set_stable_ticks(2); out.request.set_interval_ticks(1); out.request.set_max_gap_ticks(10);
    if (submit) CHECK(call(PrepareRun, out.request).has_effect_record());
    return out;
}
static Prepared start() {
    auto prepared = preparation(); CHECK(call(CommitRun, prepared.keyed(true)).owner_active()); return prepared;
}
static void update(std::uint64_t tick) { fake::tick = static_cast<std::int64_t>(tick); CHECK(plugin_onupdate(output) == CR_OK); CHECK(fake::suspension == 0); }
static void authentication_and_inventory() {
    ++scenarios; setup();
    std::unique_ptr<RPCService> service(plugin_rpcconnect(output));
    CHECK(service->names == std::vector<std::string>({"Handshake", "ObserveRun", "PrepareRun", "CommitRun", "QueryRun", "CancelRun"}));
    for (auto flag : service->flags) CHECK(flag == 0);
    const auto before = fake::tile_reads;
    auto in = base(); CHECK(call(Handshake, in).bridge_generation() == generation);
    in.set_bearer_token(std::string(32, 'x')); CHECK(call(ObserveRun, in, false).failure_code() == 1);
    in = base(); in.set_protocol_minor(13); CHECK(call(Handshake, in, false).failure_code() == 2);
    in = base(); in.set_client_nonce("short"); call(Handshake, in, false);
    in = base(); in.clear_protocol_major(); call(Handshake, in, false);
    in = base(); in.unknown.count = 1; call(Handshake, in, false);
    in = base(); in.size_override = 2049; call(Handshake, in, false);
    for (const auto *value : {"", "0", "true"}) {
        setenv("DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18", value, 1); call(Handshake, base(), false);
    }
    setenv("DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18", "1", 1);
    setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL", "1.0", 1); call(Handshake, base(), false);
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL"); CHECK(fake::tile_reads == before);
}
static void shape_and_capture_contract() {
    ++scenarios; setup();
    auto request = observation_request(); auto good = call(ObserveRun, request);
    auto capture = run::decode_capture(good.observation()); CHECK(capture.region.size() == 4);
    CHECK(capture.source.identity.folder == "region1"); CHECK(!capture.matches());
    for (std::size_t n = 0; n < good.observation().size(); ++n) {
        bool rejected = false;
        try { (void)run::decode_capture(good.observation().substr(0, n)); } catch (const run::clock::Failure &) { rejected = true; }
        CHECK(rejected);
    }
    for (const auto &bad : {good.observation() + "x", std::string(1025, 'x')}) {
        bool rejected = false; try { (void)run::decode_capture(bad); } catch (const run::clock::Failure &) { rejected = true; }
        CHECK(rejected);
    }
    request.clear_z(); call(ObserveRun, request, false);
    request = observation_request(); request.set_width(9); call(ObserveRun, request, false);
    request = observation_request(); request.set_x(63); call(ObserveRun, request, false);
    request = observation_request(); request.set_game_ticks(100); call(ObserveRun, request, false);
    request = base(); request.set_idempotency_key("a"); call(Handshake, request, false);
    const auto p = preparation(false);
    auto malformed = p.request; malformed.clear_stable_samples(); call(PrepareRun, malformed, false);
    malformed = p.request; malformed.set_prepare_token(std::string(16, 'x')); call(PrepareRun, malformed, false);
    malformed = p.request; malformed.set_plan_digest(std::string(32, 'x')); call(PrepareRun, malformed, false);
    CHECK(engine.query(p.key, p.plan) == nullptr);
}
static void hidden_noninterference() {
    ++scenarios; setup();
    auto &block = fake::blocks[0][0]; block.designation[15][15].bits = {1, 999, 999};
    block.tiletype[15][15] = static_cast<df::tiletype>(-999);
    const auto before = fake::shape_reads;
    const auto first = call(ObserveRun, observation_request()).observation();
    CHECK(fake::shape_reads - before == 3);
    auto value = run::decode_capture(first); CHECK(value.cells[0] == (run::Cell{1,0,0,0}));
    block.tiletype[15][15] = static_cast<df::tiletype>(1234); block.designation[15][15].bits.flow_size = 123;
    CHECK(call(ObserveRun, observation_request()).observation() == first);
    const auto p = preparation(false); call(PrepareRun, p.request, false);
    CHECK(engine.query(p.key, p.plan) == nullptr);
}
static void native_run_and_client_disconnect() {
    ++scenarios; setup(); const auto unpauses = fake::unpauses;
    auto p = preparation(); const auto reads = fake::tile_reads;
    const auto prepared = call(PrepareRun, p.request).effect_record(); CHECK(fake::tile_reads == reads);
    std::unique_ptr<RPCService> connection(plugin_rpcconnect(output));
    call(CommitRun, p.keyed(true)); connection.reset(); CHECK(engine.active()); CHECK(!fake::paused);
    fake::terrain(true); update(101); CHECK(p.record().stable_samples == 1);
    for (int i = 0; i < 10; ++i) { update(101); CHECK(p.record().stable_samples == 1); }
    update(102); CHECK(engine.active()); update(103); CHECK(!engine.active()); CHECK(fake::paused);
    CHECK(p.record().trigger == run::Trigger::FloorObserved); CHECK(p.record().run->pause_verified);
    auto terminal = call(QueryRun, p.keyed(false)).effect_record(); CHECK(terminal != prepared);
    const auto after_reads = fake::tile_reads;
    fake::terrain(false); fake::tick = 500; fake::paused = false;
    CHECK(call(CommitRun, p.keyed(true)).effect_record() == terminal);
    CHECK(call(CancelRun, p.keyed(true)).effect_record() == terminal);
    CHECK(fake::tile_reads == after_reads); CHECK(fake::unpauses == unpauses + 1); CHECK(!fake::paused);
}
static void ambiguous_reply_keeps_stop_owner() {
    ++scenarios; setup(); auto p = preparation(); const auto unpauses = fake::unpauses;
    fake::throw_reply = true; call(CommitRun, p.keyed(true), false); CHECK(engine.active()); CHECK(!fake::paused);
    call(CommitRun, p.keyed(true)); CHECK(fake::unpauses == unpauses + 1);
    fake::terrain(true); update(101); update(103); CHECK(!engine.active()); CHECK(fake::paused);
    CHECK(call(QueryRun, p.keyed(false)).has_effect_record());
}
static void failed_capture_pause_and_clock() {
    ++scenarios; setup(); auto p = start(); fake::fail_capture = true; update(101);
    CHECK(!engine.active()); CHECK(fake::paused); CHECK(p.record().trigger == run::Trigger::CaptureFailure);
    CHECK(p.record().run->pause_verified); CHECK(call(QueryRun, p.keyed(false)).has_effect_record());
    setup(); p = start(); fake::year = -1; update(101); CHECK(!engine.active()); CHECK(fake::paused);
    CHECK(p.record().run->pause_verified); CHECK(!p.record().run->tick_known);
    CHECK(call(QueryRun, p.keyed(false)).has_effect_record());
}
static void partial_visibility_and_liquid() {
    ++scenarios;
    for (int mode = 0; mode < 3; ++mode) {
        setup(); auto p = start();
        if (mode == 0) fake::missing[0][0] = true;
        if (mode == 1) fake::blocks[0][0].designation[15][15].bits.hidden = 1;
        if (mode == 2) fake::blocks[0][0].designation[15][15].bits.flow_size = 1;
        update(101); CHECK(!engine.active()); CHECK(fake::paused);
        CHECK(p.record().trigger == (mode == 2 ? run::Trigger::LiquidObserved : run::Trigger::Unobservable));
        CHECK(call(QueryRun, p.keyed(false)).has_effect_record());
    }
}
static void failed_pause_drain_and_limits() {
    ++scenarios; setup(); auto p = start(); fake::noop_pause = true;
    call(CancelRun, p.keyed(true)); CHECK(engine.active()); CHECK(p.record().run->phase == run::clock::Phase::Stopping);
    CHECK(plugin_shutdown(output) == CR_FAILURE);
    auto competitor = preparation(false); CHECK(call(PrepareRun, competitor.request, false).failure_code() == 8);
    fake::noop_pause = false; update(101); CHECK(!engine.active()); CHECK(p.record().run->pause_verified);
    setup(); p = start(); fake::terrain(true); update(200);
    CHECK(p.record().trigger != run::Trigger::FloorObserved); CHECK(p.record().run->reason == run::clock::Reason::TickLimit);
    setup(); p = start();
    { CoreSuspender suspend; service(p.record().run->deadline); }
    CHECK(!engine.active()); CHECK(fake::paused); CHECK(p.record().run->reason == run::clock::Reason::WallLimit);
}
static void source_changes_and_writer_fence() {
    ++scenarios;
    for (int mode = 0; mode < 4; ++mode) {
        setup(); auto p = start(); const auto pauses = fake::pauses;
        if (mode == 0) { fake::folder = "replacement"; }
        if (mode == 1) ++fake::site;
        if (mode == 2) ++fake::sz;
        if (mode == 3) CHECK(plugin_onstatechange(output, SC_MAP_UNLOADED) == CR_OK);
        update(101); CHECK(!engine.active()); CHECK(fake::pauses == pauses);
        CHECK(p.record().run->phase == run::clock::Phase::SourceLost); CHECK(!p.record().run->pause_verified);
        CHECK(call(QueryRun, p.keyed(false)).has_effect_record());
    }
    setup(); run::Identity original;
    { CoreSuspender suspend; original = read_source().identity; }
    ++fake::site; bool refused = false; const auto pauses = fake::pauses;
    { CoreSuspender suspend; try { write_native(original, true); } catch (const run::clock::Failure &) { refused = true; } }
    CHECK(refused); CHECK(fake::pauses == pauses);
}
static void clock_permission_and_stale_preparation() {
    ++scenarios; setup(); auto p = preparation(false);
    setenv("DFMCP_EXCAVATION_RUN_ALLOW_CLOCK", "0", 1); call(PrepareRun, p.request, false);
    setenv("DFMCP_EXCAVATION_RUN_ALLOW_CLOCK", "1", 1); call(PrepareRun, p.request);
    setenv("DFMCP_EXCAVATION_RUN_ALLOW_CLOCK", "0", 1); call(CommitRun, p.keyed(true), false);
    CHECK(p.record().run->phase == run::clock::Phase::Prepared); call(QueryRun, p.keyed(false));
    call(CancelRun, p.keyed(true)); CHECK(!p.record().run->unpause_attempted);
    setup(); p = preparation(); fake::blocks[0][0].designation[15][15].bits.dig = 0;
    const auto before = fake::unpauses; call(CommitRun, p.keyed(true));
    CHECK(p.record().run->phase == run::clock::Phase::Refused); CHECK(fake::unpauses == before);
    setup(); p = start(); unsetenv("DFMCP_EXCAVATION_RUN_ALLOW_CLOCK");
    unsetenv("DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18"); fake::fail_capture = true; update(101);
    CHECK(!engine.active()); CHECK(fake::paused); // Revocation never disables safety callbacks.
}
static void unload_veto_during_unpause() {
    ++scenarios; setup(); auto p = preparation(); int veto = -1;
    fake::setter_hook = [&](bool paused) {
        if (!paused) {
            const auto entries = fake::suspension_entries;
            veto = plugin_onstatechange(output, SC_BEGIN_UNLOAD);
            CHECK(fake::suspension_entries == entries);
        }
    };
    call(CommitRun, p.keyed(true)); CHECK(veto == CR_FAILURE); CHECK(engine.active());
    CHECK(plugin_shutdown(output) == CR_FAILURE);
    update(101); CHECK(!engine.active()); CHECK(fake::paused);
    CHECK(p.record().run->reason == run::clock::Reason::Shutdown);
    CHECK(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_OK); CHECK(plugin_shutdown(output) == CR_OK);
}
static std::string hex(const std::string &bytes) {
    static const char alphabet[] = "0123456789abcdef"; std::string out;
    for (unsigned char c : bytes) { out.push_back(alphabet[c >> 4]); out.push_back(alphabet[c & 15]); }
    return out;
}
static void vectors_and_maximum_size() {
    ++scenarios;
    run::Capture before;
    before.source = {{41, 3, 806500, true, true, true}, {41, 2, 64, 64, 8, "region1"}};
    before.region = {15, 15, 2, 2, 2};
    for (std::size_t i = 0; i < before.region.size(); ++i) before.cells[i] = {2,2,0,1};
    const run::clock::Spec limits{100, 1000}; const run::Goal goal{2,2,1,10};
    const auto plan = run::plan_digest(limits, goal, before);
    run::clock::Record clock_record; clock_record.key = "golden"; clock_record.plan = plan;
    clock_record.spec = limits; clock_record.before = before.source.clock;
    run::Record record; record.run = &clock_record; record.before = before; record.goal = goal;
    record.counted_tick = record.last_tick = 806500;
    const auto prepared = run::encode_record(record);
    const auto finish_record = [&] {
        clock_record.phase = run::clock::Phase::Stopped; clock_record.reason = run::clock::Reason::Cancelled;
        clock_record.unpause_attempted = clock_record.pause_verified = clock_record.tick_known = true;
        clock_record.observed_tick = 806504;
        record.trigger = run::Trigger::FloorObserved; record.stable_samples = 2; record.first_stable_tick = 806501;
        record.counted_tick = record.last_tick = 806504; record.sample = record.before;
        record.sample->source.clock.tick = 806504; record.sample->source.clock.sequence = 4; record.sample->source.clock.paused = false;
        for (std::size_t i = 0; i < record.sample->region.size(); ++i) record.sample->cells[i] = {2,3,0,0};
    };
    finish_record(); const auto stopped = run::encode_record(record);
    for (std::size_t i = 0; i < before.region.size(); ++i) {
        auto changed = before; changed.cells[i].dig = 0; CHECK(run::plan_digest(limits, goal, changed) != plan);
    }
    for (int field = 0; field < 6; ++field) {
        auto changed = goal; auto bound = limits;
        if (field == 0) ++bound.game_ticks;
        if (field == 1) ++bound.wall_ms;
        if (field == 2) ++changed.samples;
        if (field == 3) ++changed.stable_ticks;
        if (field == 4) ++changed.interval;
        if (field == 5) ++changed.max_gap;
        CHECK(run::plan_digest(bound, changed, before) != plan);
    }
    auto invalid = record; invalid.stable_samples = 0; bool refused = false;
    try { (void)run::encode_record(invalid); } catch (const run::clock::Failure &) { refused = true; }
    CHECK(refused);
    record.before.source.identity.folder = std::string(512, 'r'); record.before.region = {0,0,2,8,8};
    for (auto &cell : record.before.cells) cell = {2,2,0,1};
    clock_record.key = std::string(128, 'k'); clock_record.plan = run::plan_digest(limits, goal, record.before);
    finish_record(); const auto maximum = run::encode_record(record); CHECK(maximum.size() <= run::MAX_RECORD_BYTES);
    CHECK(run::encode_capture(record.before).size() <= run::MAX_CAPTURE_BYTES);
    auto packet = base(); wire::Reply reply; initialize_reply(packet, reply); reply.set_effect_record(maximum);
    reply.set_client_nonce(std::string(64, 'n')); reply.set_df_version(std::string(128,'v')); reply.set_dfhack_version(std::string(128,'v'));
    reply.set_owner_active(false); reply.set_retained_records(256); CHECK(reply.ByteSizeLong() <= 4096);
    std::cout << "{\"scenarios\":" << scenarios << ",\"assertions\":" << assertions << ",\"maximum_record_bytes\":" << maximum.size()
        << ",\"vectors\":{\"capture\":\"" << hex(run::encode_capture(before)) << "\",\"plan\":\"" << hex(plan)
        << "\",\"token\":\"" << hex(run::token_for("golden", plan)) << "\",\"prepared\":\"" << hex(prepared)
        << "\",\"stopped\":\"" << hex(stopped) << "\"}}\n";
}
int main() {
    try {
        authentication_and_inventory(); shape_and_capture_contract(); hidden_noninterference(); native_run_and_client_disconnect();
        ambiguous_reply_keeps_stop_owner(); failed_capture_pause_and_clock(); partial_visibility_and_liquid();
        failed_pause_drain_and_limits(); source_changes_and_writer_fence(); clock_permission_and_stale_preparation();
        unload_veto_during_unpause(); vectors_and_maximum_size();
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
