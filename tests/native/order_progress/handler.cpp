#include "stubs.h"
#include "../../../bridge/dfhack-order-progress-v1_11/dfmcp_order_progress_v1_11.cpp"
#include <functional>
#include <iostream>

namespace {
unsigned assertions = 0;
void check(bool ok) { ++assertions; if (!ok) throw std::runtime_error("assertion " + std::to_string(assertions)); }
DFHack::color_ostream output;
df::world world;
wire::Request request() {
    wire::Request r; r.set_bearer_token(std::string(32,'s')); r.set_client_nonce(std::string(16,'n'));
    r.set_protocol_major(1); r.set_protocol_minor(11); return r;
}
void initialize(df::manager_order &o) {
    o.id = 10; o.job_type = df::job_type::ConstructBed; o.item_type = df::item_type::NONE;
    o.item_subtype = -1; o.mat_type = -1; o.mat_index = -1; o.reaction_name.clear();
    o.specflag.encrust_flags.whole = 0; o.specdata.hist_figure_id = -1;
    o.material_category.whole = 0; o.material_category.bits.wood = true;
    o.art_spec.type = -1; o.art_spec.id = -1; o.art_spec.subid = -1;
    o.amount_left = 5; o.amount_total = 5; o.status.whole = 0;
    o.frequency = df::workquota_frequency_type::OneTime; o.workshop_id = -1; o.max_workshops = 1;
    o.item_conditions.clear(); o.order_conditions.clear(); o.items = nullptr;
}
wire::Reply read(std::uint32_t id) {
    auto r = request(); r.set_native_order_id(id); wire::Reply out;
    check(ReadOrderProgress(output,&r,&out) == CR_OK); return out;
}
void refusal(const wire::Request &r, std::uint32_t code) {
    wire::Reply out; out.set_observation("old");
    check(ReadOrderProgress(output,&r,&out) == CR_OK); check(!out.accepted());
    check(out.failure_code() == code); check(!out.has_observation()); check(out.bridge_generation() == 0);
}
void hex(const std::string &bytes, const char *name) {
    constexpr char digits[] = "0123456789abcdef";
    std::cout << name << '=';
    for (unsigned char b : bytes) std::cout << digits[b >> 4] << digits[b & 15];
    std::cout << '\n';
}
void observations() {
    df::manager_order o; initialize(o);
    world.manager_orders.manager_order_next_id = 11; world.manager_orders.all.assign({&o});
    reader = op::Reader(7);
    auto initial = read(10); check(initial.accepted()); check(initial.protocol_minor() == 11);
    auto raw = capture(10); check(raw.present && raw.left == 5 && raw.recipe == 1);
    hex(initial.observation(),"pending");
    o.status.whole = 3; o.amount_left = 2; ++World::tick;
    auto active = read(10); check(active.accepted()); check(capture(10).left == 2); hex(active.observation(),"active");
    o.amount_left = 0; ++World::tick;
    auto zero = read(10); check(zero.accepted()); hex(zero.observation(),"zero");
    world.manager_orders.all.clear(); ++World::tick;
    auto missing = read(10); check(missing.accepted()); hex(missing.observation(),"missing");
    check(!capture(10).present); check(!capture(100).present);
    check(world.manager_orders.manager_order_next_id == 11);
    world.manager_orders.all.assign({&o});
    for (auto pair : {std::pair{df::job_type::ConstructBed,1}, {df::job_type::ConstructDoor,2},
        {df::job_type::ConstructTable,3}, {df::job_type::ConstructThrone,4}}) {
        initialize(o); o.job_type = pair.first;
        for (int amount = 1; amount <= 100; ++amount) {
            o.amount_total = static_cast<std::int16_t>(amount);
            for (int left : {0,amount/2,amount}) {
                o.amount_left = static_cast<std::int16_t>(left);
                check(capture(10).recipe == pair.second);
            }
        }
    }
    initialize(o);
    const std::vector<std::function<void()>> corrupt = {
        [&]{o.item_type = df::item_type::BED;}, [&]{o.item_subtype = 0;}, [&]{o.mat_type = 0;},
        [&]{o.mat_index = 0;}, [&]{o.reaction_name = "anything";}, [&]{o.specflag.encrust_flags.whole = 1;},
        [&]{o.specdata.hist_figure_id = 0;}, [&]{o.material_category.whole = 3;},
        [&]{o.art_spec.type = 0;}, [&]{o.art_spec.id = 0;}, [&]{o.art_spec.subid = 0;},
        [&]{o.frequency = df::workquota_frequency_type::Daily;}, [&]{o.workshop_id = 1;},
        [&]{o.max_workshops = 2;}, [&]{o.item_conditions.push_back(1);}, [&]{o.order_conditions.push_back(1);},
        [&]{o.items = &world;}, [&]{o.amount_total = 101;}, [&]{o.amount_total = 0;},
        [&]{o.amount_left = 6;}, [&]{o.status.whole = 4;}, [&]{o.job_type = static_cast<df::job_type>(1000);}
    };
    for (const auto &change : corrupt) {
        initialize(o); change(); auto out = read(10); check(out.accepted()); check(capture(10).recipe == 0);
        check(world.manager_orders.all.size() == 1 && world.manager_orders.all[0] == &o);
        check(world.manager_orders.manager_order_next_id == 11);
    }
    initialize(o); o.finished_year = 12; o.finished_year_tick = 100;
    check(capture(10).recipe == 1); // Mutable scheduling timestamps are not drift.
    world.manager_orders.all.clear();
}
void malformed_queue() {
    df::manager_order a,b; initialize(a); initialize(b); b.id = 2;
    world.manager_orders.all.assign({&a,&b}); world.manager_orders.manager_order_next_id = 11;
    auto r = request(); r.set_native_order_id(10);
    b.id = 10; refusal(r,5); // Duplicates after selected hit still rejected.
    b.id = 11; refusal(r,5); b.id = -1; refusal(r,5);
    world.manager_orders.all.assign({&a,nullptr}); refusal(r,5);
    world.manager_orders.all.assign(4097,&a); refusal(r,5);
    world.manager_orders.all.assign({&a}); world.manager_orders.manager_order_next_id = -1; refusal(r,5);
    world.manager_orders.manager_order_next_id = 11; a.amount_left = -1; refusal(r,5);
    initialize(a); a.amount_total = -1; refusal(r,5); initialize(a);
    Core::getInstance().map_loaded = false; refusal(r,4); Core::getInstance().map_loaded = true;
    World::fortress = false; refusal(r,4); World::fortress = true;
    World::year = -1; refusal(r,5); World::year = 0;
    World::tick = 403200; refusal(r,5); World::tick = 12345;
    World::folder = std::string("a\0b",3); refusal(r,5); World::folder = "region1";
    World::folder = std::string(513,'a'); refusal(r,5); World::folder = "region1";
    world.manager_orders.all.clear();
}
void protocol() {
    auto r = request(); r.set_native_order_id(10);
    r.set_protocol_minor(10); refusal(r,2); r.set_protocol_minor(11);
    r.set_bearer_token(std::string(32,'x')); refusal(r,1); r.set_bearer_token(std::string(32,'s'));
    r.set_client_nonce("short"); refusal(r,3); r.set_client_nonce(std::string(16,'n'));
    r.unknown.present = true; refusal(r,3); r.unknown.present = false;
    r.clear_native_order_id(); refusal(r,3); r.set_native_order_id(UINT32_MAX); refusal(r,3);
    r.set_native_order_id(10); r.clear_protocol_major(); refusal(r,3); r.set_protocol_major(1);
    setenv("DFMCP_ALLOW_UNADMITTED_ORDER_PROGRESS_V1_11","0",1); refusal(r,1);
    setenv("DFMCP_ALLOW_UNADMITTED_ORDER_PROGRESS_V1_11","1",1);
    wire::Reply out; check(Handshake(output,&r,&out) == CR_OK && !out.accepted());
    r.clear_native_order_id(); check(Handshake(output,&r,&out) == CR_OK && out.accepted() && !out.has_observation());
    r.set_native_order_id(10); wire::fail_observation_reply = true;
    auto failed = read(10); check(!failed.accepted() && failed.failure_code() == 5 && !failed.has_observation());
    const auto generation = reader.generation();
    plugin_onstatechange(output,SC_PAUSED); check(reader.generation() == generation);
    plugin_onstatechange(output,SC_MAP_LOADED); check(reader.generation() != generation);
    auto service = std::unique_ptr<RPCService>(plugin_rpcconnect(output));
    check(service->names == std::vector<std::string>{"Handshake","ReadOrderProgress"});
    check(service->flags == std::vector<int>{0,0});
}
void codec() {
    op::Observation o; o.folder = "region1"; o.order = 10; o.next_order = 11; o.site = 1;
    op::Reader source(7); auto a = source.read(10,[&](std::uint32_t){return o;});
    auto b = source.read(10,[&](std::uint32_t){return o;}); check(a.sequence + 1 == b.sequence);
    source.reset(); check(source.generation() == 8);
    for (const auto &bad : {std::string("\xc0\x80",2),std::string("\xed\xa0\x80",3),std::string("\xf4\x90\x80\x80",4),std::string("\xe0\xa0",2)})
        check(!op::utf8(bad,512));
    check(op::utf8("\xe2\x98\x83",512));
    for (auto gen : {std::uint64_t{0},UINT64_MAX}) {
        op::Reader invalid(gen); check(!invalid.available());
    }
    op::Reader last(UINT64_MAX-1); last.reset(); check(!last.available()); last.reset(); check(!last.available());
    o.present = false; o.left = 1;
    bool rejected = false; try { source.read(10,[&](std::uint32_t){return o;}); } catch (const op::Failure &) { rejected = true; }
    check(rejected);
}
}
int main() {
    try {
        setenv("DFMCP_ALLOW_UNADMITTED_ORDER_PROGRESS_V1_11","1",1);
        setenv("DFMCP_ORDER_PROGRESS_TOKEN",std::string(32,'s').c_str(),1);
        df::global::world = &world;
        observations(); malformed_queue(); protocol(); codec();
        check(df::live_orders == 0);
        std::cout << "assertions=" << assertions << "\ngroups=4\n";
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
