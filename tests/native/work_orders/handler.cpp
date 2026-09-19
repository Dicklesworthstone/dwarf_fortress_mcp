#include <functional>
#include <iostream>
#include "work_orders_stubs.h"
#ifndef WORK_ORDERS_HANDLER
#error "runner must supply the actual native handler source"
#endif
#include WORK_ORDERS_HANDLER

static unsigned assertions = 0, groups = 0;
#define CHECK(x) do { ++assertions; if (!(x)) throw std::runtime_error(#x); } while (false)
template<class F> void rejects(F f) {
    bool denied = false; try { f(); } catch (const wo::Failure &) { denied = true; } CHECK(denied);
}
DFHack::color_ostream output;
df::world game;
void cleanup() { for (auto *order : game.manager_orders.all) delete order; game.manager_orders.all.clear(); }
void setup() {
    cleanup(); df::global::world = &game; engine = wo::Engine(7); df::fail_reserve = false; wire::fail_reply = false;
    game.manager_orders.manager_order_next_id = 10;
    for (int id : {6, 2}) { auto p = new df::manager_order(); p->id = id; game.manager_orders.all.push_back(p); }
    Core::getInstance().world_loaded = true; Core::getInstance().map_loaded = true;
    World::fortress = true; World::paused = true; World::year = 0; World::tick = 12345; World::site = 1; World::folder = "region1";
    CHECK(setenv("DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10", "1", 1) == 0);
    CHECK(setenv("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION", "1", 1) == 0);
    CHECK(setenv("DFMCP_WORK_ORDERS_TOKEN", std::string(32, 's').c_str(), 1) == 0);
}
wire::Request request() {
    wire::Request r; r.set_bearer_token(std::string(32, 's')); r.set_client_nonce(std::string(16, 'n'));
    r.set_protocol_major(1); r.set_protocol_minor(10); return r;
}
wire::Request preparation(unsigned recipe = 1, unsigned amount = 5, const std::string &key = "order-001") {
    auto r = request(); const auto o = engine.inspect(read_orders); r.set_idempotency_key(key);
    r.set_recipe(recipe); r.set_amount(amount); r.set_expected_witness(o.witness());
    r.set_plan_digest(wo::plan_digest({static_cast<wo::Recipe>(recipe), amount}, o.witness())); return r;
}
wire::Request commit_request(const wire::Request &prepared) {
    auto r = request(); r.set_idempotency_key(prepared.idempotency_key()); r.set_plan_digest(prepared.plan_digest());
    r.set_prepare_token(wo::prepare_token(engine.generation(), r.idempotency_key(), r.plan_digest())); return r;
}
void handshake_and_auth() {
    ++groups; setup(); auto r = request(); wire::Reply out;
    CHECK(Handshake(output, &r, &out) == CR_OK && out.accepted());
    CHECK(out.protocol_major() == 1 && out.protocol_minor() == 10 && out.bridge_generation() == 7);
    CHECK(!out.has_observation() && !out.has_effect_record() && !out.has_replayed());
    CHECK(ReadOrders(output, &r, &out) == CR_OK && out.accepted() && out.has_observation());
    CHECK(out.observation() == engine.inspect(read_orders).encode());
    const std::vector<std::function<void(wire::Request &)>> bad{
        [](auto &v) { v.clear_protocol_minor(); }, [](auto &v) { v.set_protocol_minor(9); },
        [](auto &v) { v.set_bearer_token(std::string(32, 'x')); }, [](auto &v) { v.set_bearer_token(std::string(257, 's')); },
        [](auto &v) { v.set_client_nonce("short"); }, [](auto &v) { v.set_client_nonce(std::string(65, 'n')); },
        [](auto &v) { v.unknown.present = true; }, [](auto &v) { v.set_recipe(1); }};
    for (const auto &change : bad) {
        auto b = request(); change(b); CHECK(Handshake(output, &b, &out) == CR_OK && !out.accepted());
        CHECK(out.bridge_generation() == 0 && out.df_version().empty() && !out.has_observation() && !out.has_effect_record());
    }
    CHECK(setenv("DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10", "true", 1) == 0);
    CHECK(Handshake(output, &r, &out) == CR_OK && !out.accepted() && out.failure_code() == 1);
    CHECK(game.manager_orders.all.size() == 2);
}
void actual_creation_and_lost_reply() {
    ++groups;
    for (unsigned recipe = 1; recipe <= 4; ++recipe) for (unsigned amount : {1u, 100u}) {
        setup(); auto p = preparation(recipe, amount); wire::Reply out;
        CHECK(PrepareOrder(output, &p, &out) == CR_OK && out.accepted() && !out.replayed());
        CHECK(game.manager_orders.all.size() == 2 && game.manager_orders.manager_order_next_id == 10);
        CHECK(PrepareOrder(output, &p, &out) == CR_OK && out.replayed());
        auto c = commit_request(p); CHECK(CommitOrder(output, &c, &out) == CR_OK && out.accepted());
        const auto receipt = out.effect_record();
        CHECK(engine.query(p.idempotency_key(), p.plan_digest())->state == wo::State::Created);
        CHECK(game.manager_orders.all.size() == 3 && game.manager_orders.manager_order_next_id == 11 && World::paused);
        auto *made = game.manager_orders.all.back(); CHECK(made->id == 10 && made->amount_total == static_cast<std::int16_t>(amount) && made->amount_left == static_cast<std::int16_t>(amount));
        CHECK(made->job_type == native_recipe(static_cast<wo::Recipe>(recipe)));
        CHECK(made->material_category.bits.wood && made->status.whole == 0 && made->workshop_id == -1 && made->max_workshops == 1);
        CHECK(CommitOrder(output, &c, &out) == CR_OK && out.effect_record() == receipt && game.manager_orders.all.size() == 3);
        auto q = c; q.clear_prepare_token(); CHECK(QueryOrder(output, &q, &out) == CR_OK && out.effect_record() == receipt);
    }
    setup(); auto p = preparation(); wire::Reply out; CHECK(PrepareOrder(output, &p, &out) == CR_OK && out.accepted());
    auto c = commit_request(p); wire::fail_reply = true;
    CHECK(CommitOrder(output, &c, &out) == CR_OK && !out.accepted()); CHECK(game.manager_orders.all.size() == 3);
    CHECK(CommitOrder(output, &c, &out) == CR_OK && out.accepted()); CHECK(game.manager_orders.all.size() == 3);
    CHECK(engine.query(p.idempotency_key(), p.plan_digest())->state == wo::State::Created);
}
void authority_shape_and_staleness() {
    ++groups; setup(); auto p = preparation(); wire::Reply out;
    CHECK(setenv("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION", "0", 1) == 0);
    CHECK(PrepareOrder(output, &p, &out) == CR_OK && !out.accepted() && engine.size() == 0);
    auto r = request(); CHECK(ReadOrders(output, &r, &out) == CR_OK && out.accepted());
    CHECK(setenv("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION", "1", 1) == 0);
    const std::vector<std::function<void(wire::Request &)>> bad{
        [](auto &v) { v.set_recipe(257); }, [](auto &v) { v.set_recipe(0); }, [](auto &v) { v.set_amount(0); },
        [](auto &v) { v.set_amount(101); }, [](auto &v) { v.set_amount(UINT32_MAX); },
        [](auto &v) { v.set_prepare_token(std::string(16, 't')); }, [](auto &v) { v.clear_amount(); },
        [](auto &v) { v.set_expected_witness(std::string(32, 'x')); }, [](auto &v) { v.set_idempotency_key("bad/key"); }};
    for (const auto &change : bad) { auto b = p; change(b); CHECK(PrepareOrder(output, &b, &out) == CR_OK && !out.accepted()); CHECK(engine.size() == 0); }
    CHECK(PrepareOrder(output, &p, &out) == CR_OK && out.accepted()); auto c = commit_request(p);
    CHECK(setenv("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION", "0", 1) == 0);
    CHECK(CommitOrder(output, &c, &out) == CR_OK && !out.accepted() && game.manager_orders.all.size() == 2);
    CHECK(setenv("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION", "1", 1) == 0);
    World::tick++; CHECK(CommitOrder(output, &c, &out) == CR_OK && out.accepted());
    CHECK(engine.query(p.idempotency_key(), p.plan_digest())->state == wo::State::Refused && game.manager_orders.all.size() == 2);
    setup(); World::paused = false; p = preparation(); CHECK(PrepareOrder(output, &p, &out) == CR_OK && !out.accepted());
    setup(); game.manager_orders.all.back()->id = 6; r = request(); CHECK(ReadOrders(output, &r, &out) == CR_OK && !out.accepted());
    setup(); game.manager_orders.all.push_back(nullptr); CHECK(ReadOrders(output, &r, &out) == CR_OK && !out.accepted()); game.manager_orders.all.pop_back();
    setup(); World::tick = 403200; CHECK(ReadOrders(output, &r, &out) == CR_OK && !out.accepted());
    setup(); Core::getInstance().map_loaded = false; CHECK(ReadOrders(output, &r, &out) == CR_OK && !out.accepted());
}
void full_template_and_allocation_failure() {
    ++groups; setup(); const wo::Spec spec{wo::Recipe::WoodenBed, 5};
    const std::vector<std::function<void(df::manager_order &)>> changes{
        [](auto &a) { ++a.id; }, [](auto &a) { a.job_type = df::job_type::ConstructDoor; },
        [](auto &a) { a.item_type = df::item_type::BED; }, [](auto &a) { a.item_subtype = 0; },
        [](auto &a) { a.reaction_name = "CUSTOM"; }, [](auto &a) { a.mat_type = 0; }, [](auto &a) { a.mat_index = 0; },
        [](auto &a) { a.specflag.encrust_flags.whole = 1; }, [](auto &a) { a.specdata.hist_figure_id = 0; },
        [](auto &a) { a.material_category.whole ^= 2; }, [](auto &a) { ++a.art_spec.type; },
        [](auto &a) { a.art_spec.id = 0; }, [](auto &a) { a.art_spec.subid = 0; },
        [](auto &a) { --a.amount_left; }, [](auto &a) { --a.amount_total; }, [](auto &a) { a.status.whole = 1; },
        [](auto &a) { a.frequency = df::workquota_frequency_type::Daily; }, [](auto &a) { a.finished_year = 0; },
        [](auto &a) { a.finished_year_tick = 0; }, [](auto &a) { a.workshop_id = 0; }, [](auto &a) { a.max_workshops = 0; },
        [](auto &a) { a.item_conditions.push_back(1); }, [](auto &a) { a.order_conditions.push_back(1); },
        [](auto &a) { a.items = &a; }};
    for (const auto &change : changes) {
        setup(); create_order(10, spec); CHECK(verify_order(10, spec) == wo::configuration(10, spec));
        change(*game.manager_orders.all.back()); rejects([&] { (void)verify_order(10, spec); });
    }
    setup(); auto p = preparation(); wire::Reply out; CHECK(PrepareOrder(output, &p, &out) == CR_OK && out.accepted());
    const auto live = df::live_orders; df::fail_reserve = true; auto c = commit_request(p);
    CHECK(CommitOrder(output, &c, &out) == CR_OK && out.accepted());
    CHECK(engine.query(p.idempotency_key(), p.plan_digest())->state == wo::State::Unknown);
    CHECK(game.manager_orders.all.size() == 2 && game.manager_orders.manager_order_next_id == 10 && df::live_orders == live);
    df::fail_reserve = false; CHECK(CommitOrder(output, &c, &out) == CR_OK && out.accepted()); CHECK(game.manager_orders.all.size() == 2);
    auto other = preparation(1, 5, "other"); CHECK(PrepareOrder(output, &other, &out) == CR_OK && !out.accepted() && out.failure_code() == 8);
}
void lifecycle_and_registration() {
    ++groups; setup(); auto p = preparation(); wire::Reply out; CHECK(PrepareOrder(output, &p, &out) == CR_OK && out.accepted());
    auto c = commit_request(p); CHECK(plugin_onstatechange(output, SC_UNPAUSED) == CR_OK); CHECK(plugin_onstatechange(output, SC_PAUSED) == CR_OK);
    CHECK(CommitOrder(output, &c, &out) == CR_OK && out.accepted()); CHECK(engine.query(p.idempotency_key(), p.plan_digest())->state == wo::State::Refused);
    const auto gen = engine.generation(); CHECK(plugin_onstatechange(output, SC_MAP_UNLOADED) == CR_OK);
    CHECK(engine.generation() == gen + 1 && engine.size() == 0);
    auto q = c; q.clear_prepare_token(); CHECK(QueryOrder(output, &q, &out) == CR_OK && out.accepted() && !out.has_effect_record());
    std::unique_ptr<RPCService> service(plugin_rpcconnect(output));
    CHECK(service->names == std::vector<std::string>({"Handshake", "ReadOrders", "PrepareOrder", "CommitOrder", "QueryOrder"}));
    CHECK(service->flags == std::vector<int>({0, 0, 0, 0, 0}));
    std::vector<PluginCommand> commands; CHECK(plugin_init(output, commands) == CR_OK && commands.empty());
    CHECK(plugin_shutdown(output) == CR_OK && engine.size() == 0);
}
int main() {
    try { handshake_and_auth(); actual_creation_and_lost_reply(); authority_shape_and_staleness(); full_template_and_allocation_failure(); lifecycle_and_registration();
        cleanup(); CHECK(df::live_orders == 0); std::cout << "handler_groups=" << groups << "\nhandler_assertions=" << assertions << '\n'; return 0; }
    catch (const std::exception &e) { std::cerr << e.what() << '\n'; cleanup(); return 1; }
}
