#include "stubs.h"
#include "../../../bridge/dfhack-work-order-progress-v1_12/dfmcp_work_order_progress_v1_12.cpp"
#include <functional>
#include <iomanip>
#include <iostream>
#include <memory>
#include <sstream>
static unsigned checks = 0;
void check(bool yes) { ++checks; if (!yes) throw std::runtime_error("failed check " + std::to_string(checks)); }
std::string hex(const std::string &bytes) {
    std::ostringstream out; out << std::hex << std::setfill('0');
    for (unsigned char c : bytes) out << std::setw(2) << static_cast<unsigned>(c);
    return out.str();
}
struct Fixture {
    df::world world;
    df::manager_order order;
    DFHack::color_ostream output;
    wire::Request request;
    Fixture() {
        setenv("DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12", "1", 1);
        setenv("DFMCP_WORK_ORDER_PROGRESS_TOKEN", std::string(32,'x').c_str(), 1);
        order.id = 3; order.material_category.bits.wood = true;
        world.manager_orders.all = {&order}; df::global::world = &world;
        Core::getInstance().loaded = true; Core::getInstance().map = true;
        World::fortress = true; World::year = 0; World::tick = 12345; World::site = 1;
        World::folder = "region1"; World::paused = true; sequence = wp::Sequence(7);
        request.ids = {3, 8};
    }
    wire::Reply read() {
        wire::Reply reply; check(ReadObservation(output, &request, &reply) == CR_OK); return reply;
    }
    void refused() {
        auto reply = read(); check(!reply.accepted && !reply.has_observation && reply.generation == 0);
    }
};
void progression() {
    Fixture f; auto before = capture({3,8});
    check(before.rows.size() == 2 && before.rows[0].recipe == 1 && !before.rows[1].present);
    check(before.rows[0].left == 5 && before.rows[0].status == 0);
    f.order.status.whole = 3; f.order.amount_left = 3; ++World::tick;
    auto next = capture({3,8}); check(next.rows[0].left == 3 && next.rows[0].status == 3 && next.sequence > before.sequence);
    check(before.rows[0].left == 5); // owned copies, never cached native pointers
    f.order.amount_left = 0; auto zero = capture({3,8}); check(zero.rows[0].recipe == 1 && zero.rows[0].left == 0);
    f.world.manager_orders.all.clear(); auto absent = capture({3,8});
    check(!absent.rows[0].present && absent.queue_count == 0);
    check(f.world.manager_orders.manager_order_next_id == 10); // reads never allocate IDs
}
void recognition() {
    for (auto pair : std::vector<std::pair<df::job_type,int>>{{df::job_type::ConstructBed,1},{df::job_type::ConstructDoor,2},
            {df::job_type::ConstructTable,3},{df::job_type::ConstructThrone,4}}) {
        Fixture f; f.order.job_type = pair.first;
        for (int amount = 1; amount <= 100; ++amount) {
            f.order.amount_total = static_cast<std::int16_t>(amount);
            for (int left : {0, amount/2, amount}) {
                f.order.amount_left = static_cast<std::int16_t>(left); check(template_recipe(f.order) == pair.second);
            }
        }
    }
    std::vector<std::function<void(df::manager_order&)>> corrupt = {
        [](auto &a){a.item_type=df::item_type::BED;}, [](auto&a){a.item_subtype=0;},
        [](auto&a){a.reaction_name="other";}, [](auto&a){a.mat_type=0;}, [](auto&a){a.mat_index=0;},
        [](auto&a){a.specflag.encrust_flags.whole=1;}, [](auto&a){a.specdata.hist_figure_id=0;},
        [](auto&a){a.material_category.whole=0;}, [](auto&a){a.material_category.whole|=2;},
        [](auto&a){a.art_spec.type=1;}, [](auto&a){a.art_spec.id=0;}, [](auto&a){a.art_spec.subid=0;},
        [](auto&a){a.amount_total=0;}, [](auto&a){a.amount_total=101;}, [](auto&a){a.amount_left=-1;},
        [](auto&a){a.amount_left=6;}, [](auto&a){a.status.whole=4;},
        [](auto&a){a.frequency=df::workquota_frequency_type::Daily;}, [](auto&a){a.workshop_id=0;},
        [](auto&a){a.max_workshops=0;}, [](auto&a){a.item_conditions.push_back(nullptr);},
        [](auto&a){a.order_conditions.push_back(nullptr);}, [](auto&a){a.items=&a;}
    };
    for (auto change : corrupt) { Fixture f; change(f.order); check(template_recipe(f.order)==0); check(f.read().accepted); }
    Fixture f; f.order.finished_year=250; f.order.finished_year_tick=5; f.order.status.whole=3;
    check(template_recipe(f.order)==1); // dynamic progress is not a configuration change
}
void queue_integrity() {
    {Fixture f; f.world.manager_orders.all.push_back(&f.order); f.refused();}
    {Fixture f; f.world.manager_orders.all.push_back(nullptr); f.refused();}
    {Fixture f; f.order.id=10; f.refused();}
    {Fixture f; f.order.id=-1; f.refused();}
    {Fixture f; f.world.manager_orders.manager_order_next_id=-1; f.refused();}
    {Fixture f; f.request.ids={8,3}; f.refused();}
    {Fixture f; f.request.ids={3,3}; f.refused();}
    {Fixture f; f.request.ids.clear(); f.refused();}
    {Fixture f; f.request.ids={UINT32_MAX}; f.refused();}
    {Fixture f; f.request.ids.resize(33); f.refused();}
    {Fixture f; f.world.manager_orders.all.resize(wp::MAX_QUEUE+1,&f.order); f.refused();}
    Fixture f; std::vector<df::manager_order> queue(wp::MAX_QUEUE);
    f.world.manager_orders.all.clear(); f.world.manager_orders.manager_order_next_id=4096;
    f.request.ids.clear();
    for (std::size_t i=0;i<queue.size();++i) {
        queue[i]=f.order; queue[i].id=static_cast<std::int32_t>(i); f.world.manager_orders.all.push_back(&queue[i]);
        if (i<32) f.request.ids.push_back(static_cast<std::uint32_t>(i));
    }
    std::reverse(f.world.manager_orders.all.begin(),f.world.manager_orders.all.end());
    check(f.read().accepted);
    queue[4095].id=0; f.refused(); // duplicate outside selection invalidates absence evidence too
}
void auth_and_lifecycle() {
    for (int mode=0;mode<8;++mode) {
        Fixture f;
        if(mode==0) f.request.token="short";
        if(mode==1) f.request.token=std::string(32,'y');
        if(mode==2) f.request.nonce="short";
        if(mode==3) f.request.minor=10;
        if(mode==7) f.request.minor=11; // Existing single-order progress generation is not this wire.
        if(mode==4) f.request.unknown=true;
        if(mode==5) f.request.initialized=false;
        if(mode==6) setenv("DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12","0",1);
        f.refused();
    }
    for (auto event : {SC_MAP_LOADED,SC_MAP_UNLOADED,SC_WORLD_LOADED,SC_WORLD_UNLOADED}) {
        Fixture f; auto a=capture({3}); plugin_onstatechange(f.output,event); auto b=capture({3});
        check(a.generation!=b.generation && b.sequence==1);
    }
    {Fixture f; Core::getInstance().loaded=false; f.refused();}
    {Fixture f; Core::getInstance().map=false; f.refused();}
    {Fixture f; World::fortress=false; f.refused();}
    {Fixture f; World::year=-1; f.refused();}
    {Fixture f; World::tick=403200; f.refused();}
    {Fixture f; World::year=1; World::tick=-1; f.refused();}
    {Fixture f; World::folder=std::string("a\0b",3); f.refused();}
    {Fixture f; f.order.reaction_name=std::string(129,'a'); f.refused();}
    {Fixture f; f.order.reaction_name=std::string("\xc0\x80",2); f.refused();}
    {Fixture f; sequence=wp::Sequence(UINT64_MAX); f.refused(); sequence.reset(); f.refused();}
    {Fixture f; wire::Reply out; out.fail_observation=true;
        check(ReadObservation(f.output,&f.request,&out)==CR_OK); check(!out.accepted && !out.has_observation);
        check(f.order.amount_left==5 && f.world.manager_orders.all.size()==1); check(f.read().accepted);}
    Fixture f; f.request.ids.clear(); wire::Reply reply;
    check(Handshake(f.output,&f.request,&reply)==CR_OK && reply.accepted && !reply.has_observation);
    f.request.ids={3}; check(Handshake(f.output,&f.request,&reply)==CR_OK && !reply.accepted);
    std::unique_ptr<RPCService> service(plugin_rpcconnect(f.output));
    check(service->methods==std::vector<std::pair<std::string,int>>{{"Handshake",0},{"ReadObservation",0}});
}
void bounds_and_vector() {
    Fixture f; wp::Observation maximal; maximal.generation=7; maximal.sequence=1; maximal.tick=0;
    maximal.folder=std::string(512,'f'); maximal.next_order=100; maximal.queue_count=32;
    for(std::uint32_t i=0;i<32;++i) {
        wp::Row r; r.id=i; r.present=true; r.type_key=std::string(128,'t'); r.reaction=std::string(128,'r');
        maximal.rows.push_back(r);
    }
    check(maximal.encode().size()<=wp::MAX_BYTES);
    wp::Row absent; absent.id=3; check(absent.encode().size()==5); absent.left=1;
    try { (void)absent.encode(); check(false); } catch(const wp::Failure&) {check(true);}
    f.order.amount_left=3; f.order.status.whole=3; sequence=wp::Sequence(7);
    auto reply=f.read(); check(reply.accepted);
    std::cout<<"VECTOR "<<hex(reply.observation)<<'\n';
}
int main() {
    try { progression(); recognition(); queue_integrity(); auth_and_lifecycle(); bounds_and_vector();
        std::cout<<"PASS "<<checks<<" assertions 5 groups\n"; return 0;
    } catch(const std::exception &e) { std::cerr<<e.what()<<'\n'; return 1; }
}
