#include "bridge/dfhack-order-run-v1_14/dfmcp_order_run_v1_14.cpp"
#include <iostream>
#include <stdexcept>
#include <thread>
namespace {
int checks = 0, keys = 0;
void check_impl(bool v, int line) { ++checks; if (!v) throw std::runtime_error("line " + std::to_string(line)); }
#define check(v) check_impl((v), __LINE__)
color_ostream output;
wire::Request request() {
    wire::Request r; r.set_bearer_token(std::string(32, 'x')); r.set_client_nonce(std::string(16, 'n'));
    r.set_protocol_major(1); r.set_protocol_minor(14); return r;
}
std::string hex(const std::string &s) {
    const char *digits = "0123456789abcdef"; std::string out;
    for (unsigned char c : s) { out += digits[c >> 4]; out += digits[c & 15]; }
    return out;
}
struct Fixture {
    df::world world; df::manager_order order;
    wire::Request planned;
    Fixture() {
        check(!engine.active()); Core::getInstance().loaded = Core::getInstance().map = true;
        World::fortress = World::paused = true; World::year = 0; World::tick = 100; World::site = 7; World::folder = "fort";
        World::fail_pause = World::fail_after_unpause = false; World::during_unpause = nullptr;
        order.id = 9; order.material_category.bits.wood = true; world.manager_orders.all = {&order}; df::global::world = &world;
        gate.store(Gate::Idle); drain_requested.store(false);
    }
    ~Fixture() {
        plugin_onstatechange(output, SC_MAP_UNLOADED); df::global::world = nullptr;
    }
    wire::Reply observe(std::uint32_t id = 9) {
        auto r = request(); r.set_native_order_id(id); wire::Reply out;
        check(ObserveRun(output, &r, &out) == CR_OK); return out;
    }
    wire::Reply prepare(run::Goal goal = {}, run::clock::Spec limits = {100, 1000}, std::string key = "") {
        auto observed = observe(); check(observed.accepted());
        auto capture = run::decode_capture(observed.observation());
        planned = request(); planned.set_idempotency_key(key.empty() ? "case-" + std::to_string(++keys) : key);
        planned.set_game_ticks(limits.game_ticks); planned.set_wall_ms(limits.wall_ms);
        planned.set_expected_capture(observed.observation()); planned.set_plan_digest(run::plan_digest(limits, goal, capture));
        planned.set_native_order_id(9); planned.set_predicate(static_cast<std::uint32_t>(goal.predicate));
        planned.set_threshold(goal.threshold); planned.set_stable_samples(goal.samples); planned.set_interval_ticks(goal.interval);
        wire::Reply out; check(PrepareRun(output, &planned, &out) == CR_OK); return out;
    }
    wire::Request keyed(bool token) {
        auto r = request(); r.set_idempotency_key(planned.idempotency_key()); r.set_plan_digest(planned.plan_digest());
        if (token) r.set_prepare_token(run::token_for(r.idempotency_key(), r.plan_digest()));
        return r;
    }
    wire::Reply commit() { auto r = keyed(true); wire::Reply out; check(CommitRun(output, &r, &out) == CR_OK); return out; }
    wire::Reply query() { auto r = keyed(false); wire::Reply out; check(QueryRun(output, &r, &out) == CR_OK); return out; }
    wire::Reply cancel() { auto r = keyed(true); wire::Reply out; check(CancelRun(output, &r, &out) == CR_OK); return out; }
    const run::Record &record() { auto *r = engine.query(planned.idempotency_key(), planned.plan_digest()); check(r); return *r; }
    void step(int ticks = 1) { World::tick += ticks; check(plugin_onupdate(output) == CR_OK); }
};
void canonical_vectors() {
    generation = 41; Fixture f; auto observed = f.observe(); check(observed.accepted());
    auto before = run::decode_capture(observed.observation());
    auto prepared = f.prepare({}, {100, 1000}, "vector"); check(prepared.accepted());
    std::cout << "capture=" << hex(observed.observation()) << '\n';
    std::cout << "plan=" << hex(run::plan_bytes({100,1000}, {}, before)) << '\n';
    std::cout << "record=" << hex(prepared.effect_record()) << '\n';
    check(f.cancel().accepted()); check(World::unpauses == 0);
}
void authentication_and_shapes() {
    Fixture f; int reads = World::reads;
    for (int mode = 0; mode < 7; ++mode) {
        auto r = request(); wire::Reply out;
        if (mode == 0) r.set_bearer_token(std::string(32,'z'));
        if (mode == 1) r.set_protocol_minor(13);
        if (mode == 2) r.set_client_nonce("short");
        if (mode == 3) r.unknown = true;
        if (mode == 4) r.forced_size = 2049;
        if (mode == 5) r.set_native_order_id(9);
        if (mode == 6) r.p_bearer_token = false;
        check(Handshake(output, &r, &out) == CR_OK && !out.accepted());
    }
    check(World::reads == reads);
    auto r = request(); wire::Reply out; r.set_native_order_id(UINT32_MAX);
    check(ObserveRun(output, &r, &out) == CR_OK && !out.accepted());
    auto service = std::unique_ptr<RPCService>(plugin_rpcconnect(output));
    check(service->methods.size() == 6);
    const std::vector<std::string> names{"Handshake","ObserveRun","PrepareRun","CommitRun","QueryRun","CancelRun"};
    for (std::size_t i = 0; i < names.size(); ++i) check(service->methods[i] == std::make_pair(names[i], 0));
    check(f.prepare().accepted());
    r = f.planned; r.set_predicate(257); check(PrepareRun(output, &r, &out) == CR_OK && !out.accepted());
    for (std::size_t i = 0; i < f.planned.expected_capture().size(); ++i) {
        r = f.planned; auto bytes = r.expected_capture(); bytes[i] ^= 1; r.set_expected_capture(bytes);
        check(PrepareRun(output, &r, &out) == CR_OK && !out.accepted());
    }
    check(f.cancel().accepted());
}
void predicate_callbacks_and_disconnected_clients() {
    for (auto kind : {run::Predicate::Approved, run::Predicate::Active, run::Predicate::RemainingAtMost}) {
        Fixture f; const auto before = World::unpauses;
        check(f.prepare({kind, kind == run::Predicate::RemainingAtMost ? 5u : 0u, 2, 1}).accepted());
        auto service = std::unique_ptr<RPCService>(plugin_rpcconnect(output));
        check(f.commit().accepted()); service.reset(); // Drop RPC ownership, not the clock owner.
        check(engine.active());
        f.order.status.whole = kind == run::Predicate::Approved ? 1u : kind == run::Predicate::Active ? 2u : 0u;
        if (kind == run::Predicate::RemainingAtMost) f.order.amount_left = 5;
        check(f.query().accepted() && !f.record().predicate_observed()); // Query does not service or mutate.
        f.step(); check(f.record().stable_samples == 1 && engine.active());
        f.step(0); check(f.record().stable_samples == 1 && engine.active());
        f.step(); check(f.record().predicate_observed() && f.record().run->pause_verified && !engine.active());
        auto saved = f.query().effect_record(); f.order.status.whole = 0; f.step();
        check(f.query().effect_record() == saved); check(f.commit().accepted() && World::unpauses == before + 1);
    }
}
void mutate(df::manager_order &a, int n) {
    static int dummy;
    switch (n) {
        case 0: a.item_type = df::item_type::BED; break;
        case 1: a.item_subtype = 0; break;
        case 2: a.reaction_name = "OTHER"; break;
        case 3: a.mat_type = 1; break;
        case 4: a.mat_index = 1; break;
        case 5: a.specflag.encrust_flags.whole = 1; break;
        case 6: a.specdata.hist_figure_id = 1; break;
        case 7: a.material_category.whole = 0; break;
        case 8: a.art_spec.type = 1; break;
        case 9: a.art_spec.id = 1; break;
        case 10: a.art_spec.subid = 1; break;
        case 11: a.amount_total = 11; break;
        case 12: a.frequency = df::workquota_frequency_type::Daily; break;
        case 13: a.workshop_id = 1; break;
        case 14: a.max_workshops = 2; break;
        case 15: a.item_conditions.push_back(&dummy); break;
        case 16: a.order_conditions.push_back(&dummy); break;
        case 17: a.items = &dummy; break;
        case 18: a.status.whole = 4; break;
        case 19: a.job_type = df::job_type::ConstructDoor; break;
    }
}
void target_drift_and_queue_validation() {
    for (int n = 0; n < 20; ++n) {
        Fixture f; check(f.prepare().accepted() && f.commit().accepted()); mutate(f.order, n); f.step();
        check(f.record().trigger == run::Trigger::TargetChanged && !f.record().predicate_observed());
        check(f.record().run->pause_verified);
    }
    Fixture f; check(f.prepare().accepted() && f.commit().accepted()); f.world.manager_orders.all.clear(); f.step();
    check(f.record().trigger == run::Trigger::TargetAbsent && f.record().run->pause_verified);
    for (int n = 0; n < 4; ++n) {
        f.world.manager_orders.all = {&f.order}; f.order.id = 9; f.world.manager_orders.manager_order_next_id = 10;
        if (n == 0) f.world.manager_orders.all.push_back(nullptr);
        if (n == 1) f.world.manager_orders.all.push_back(&f.order);
        if (n == 2) f.order.id = -1;
        if (n == 3) f.world.manager_orders.manager_order_next_id = 9;
        check(!f.observe(8).accepted()); // Bad complete queue cannot prove selected absence.
    }
}
void stale_source_and_ambiguous_responses() {
    for (int n = 0; n < 3; ++n) {
        Fixture f; check(f.prepare().accepted() && f.commit().accepted()); const int pauses = World::pauses;
        if (n == 0) World::folder = "other";
        if (n == 1) ++World::site;
        if (n == 2) plugin_onstatechange(output, SC_MAP_LOADED);
        f.step(); check(f.record().run->phase == run::clock::Phase::SourceLost);
        check(World::pauses == pauses && !f.record().run->pause_verified);
    }
    Fixture f; check(f.prepare().accepted()); wire::Reply::fail_record = 1;
    const int unpauses = World::unpauses; check(!f.commit().accepted() && engine.active());
    check(f.query().accepted() && f.record().run->unpause_attempted);
    f.order.status.whole = 1; f.step(); check(f.record().run->pause_verified);
    check(f.commit().accepted() && World::unpauses == unpauses + 1);
}
void independent_setter_scope() {
    Fixture f; CoreSuspender suspended;
    const auto expected = dfmcp_order_run_native::identity(generation);
    const int writes = World::unpauses;
    for (int mode = 0; mode < 2; ++mode) {
        World::folder = mode == 0 ? "replacement" : expected.folder;
        World::site = mode == 1 ? expected.site + 1 : expected.site;
        bool refused = false;
        try { write_native(expected, false); } catch (const run::clock::Failure &) { refused = true; }
        check(refused && World::unpauses == writes);
    }
}
void unload_drain() {
    Fixture f; check(f.prepare().accepted());
    World::during_unpause = [] { check(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_FAILURE); };
    check(f.commit().accepted()); World::during_unpause = nullptr;
    World::fail_pause = true; f.step(); check(engine.active() && f.record().run->phase == run::clock::Phase::Stopping);
    check(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_FAILURE);
    World::fail_pause = false; f.step(); check(f.record().run->pause_verified);
    check(plugin_onstatechange(output, SC_BEGIN_UNLOAD) == CR_OK && plugin_shutdown(output) == CR_OK);
}
void invalid_clocks_and_maximum_wire() {
    Fixture f; check(f.prepare().accepted() && f.commit().accepted()); World::tick = -1;
    check(plugin_onupdate(output) == CR_OK && World::paused && engine.active());
    World::tick = 101; f.step(0); check(!engine.active() && f.record().run->pause_verified);
    World::folder = std::string(512, 'w'); auto obs = f.observe(); check(obs.accepted() && obs.observation().size() == run::MAX_CAPTURE_BYTES);
    f.order.status.whole = 0; auto p = f.prepare({}, {100,1000}, std::string(128,'k')); check(p.accepted());
    check(f.commit().accepted()); f.order.status.whole = 1; f.step();
    auto record = f.query().effect_record(); check(record.size() == run::MAX_RECORD_BYTES);
    World::folder = std::string("bad\xc0\xaf"); check(!f.observe().accepted());
}
}
int main() {
    setenv("DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14", "1", 1); setenv("DFMCP_ORDER_RUN_TOKEN", std::string(32,'x').c_str(), 1);
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    canonical_vectors(); authentication_and_shapes(); predicate_callbacks_and_disconnected_clients();
    target_drift_and_queue_validation(); stale_source_and_ambiguous_responses(); independent_setter_scope(); unload_drain(); invalid_clocks_and_maximum_wire();
    check(suspension_depth == 0);
    std::cout << "order-run bridge: " << checks << " assertions passed\n";
}
