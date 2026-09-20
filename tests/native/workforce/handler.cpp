#include "bridge/dfhack-workforce-v1_17/dfmcp_workforce_v1_17.cpp"
#include <iostream>
namespace {
unsigned checks=0;
void check(bool b) { ++checks; if (!b) throw std::runtime_error("workforce handler assertion failed at "+std::to_string(checks)); }
struct Scene {
    df::world world; df::plotinfost plot; df::gamest game;
    df::unit a,b; df::work_detail mine,carpenter;
    color_ostream output;
    Scene() {
        setenv("DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17","1",1);
        setenv("DFMCP_WORKFORCE_TOKEN",std::string(32,'t').c_str(),1);
        setenv("DFMCP_WORKFORCE_ALLOW_LABOR","1",1); unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
        Core::getInstance().loaded=Core::getInstance().map=true;
        World::fortress=World::paused=true; World::year=0; World::tick=100; World::site=7; World::folder="region1";
        Units::calls=0; Units::hook={};
        a.id=2;a.hist_figure_id=102;a.status.labors={false,true,false};
        b.id=5;b.hist_figure_id=105;b.status.labors={true,false,false}; world.units.active={&a,&b};
        mine.name="Miners";mine.allowed_labors={true,false,false};mine.assigned_units={5};
        carpenter.name="Carpenters";carpenter.allowed_labors={false,true,false};carpenter.assigned_units={2};
        plot.labor_info.work_details={&mine,&carpenter};
        df::global::world=&world;df::global::plotinfo=&plot;df::global::game=&game;
        plugin_onstatechange(output,SC_MAP_LOADED);
    }
    wire::Request prepared_request(std::string key="assign") {
        wire::Request r;r.ids={2,5};r.key=key;r.mask=95;
        CoreSuspender guard;const auto obs=engine.observe(r.ids,read_native);
        r.witness=obs.witness();r.plan=wf::plan_digest({0,true},r.witness);r.prepare=wf::token_for(r.key,r.plan);return r;
    }
    wire::Request prepare() {
        auto r=prepared_request();wire::Reply out;check(PrepareAssignment(output,&r,&out)==CR_OK&&out.accepted);
        return r;
    }
    void commit(wire::Request &r,wire::Reply &out) {r.ids.clear();r.mask=49;check(CommitAssignment(output,&r,&out)==CR_OK);}
};
void lifecycle() {
    Scene s;auto r=s.prepare();wire::Reply out;s.commit(r,out);
    check(out.accepted&&!out.unresolved&&out.effect_present&&Units::calls==1);
    check(s.mine.assigned_units==std::vector<std::int32_t>({2,5}));check(s.a.status.labors[0]&&s.a.status.labors[1]);
    check(s.b.status.labors[0]&&!s.b.status.labors[1]);
    auto record=out.effect;s.commit(r,out);check(out.effect==record&&Units::calls==1);
    r.mask=17;check(QueryAssignment(s.output,&r,&out)==CR_OK&&out.accepted&&out.effect==record);
    r.mask=49;check(CancelAssignment(s.output,&r,&out)==CR_OK&&out.effect==record);
    std::unique_ptr<RPCService> rpc(plugin_rpcconnect(s.output));
    check(rpc->names==std::vector<std::string>({"Handshake","ObserveWorkforce","PrepareAssignment","CommitAssignment","QueryAssignment","CancelAssignment"}));
    plugin_onstatechange(s.output,SC_WORLD_UNLOADED);r.mask=17;
    check(QueryAssignment(s.output,&r,&out)==CR_OK&&out.accepted&&!out.effect_present);
    Scene t;auto q=t.prepare();q.ids.clear();q.mask=49;
    check(CancelAssignment(t.output,&q,&out)==CR_OK&&out.accepted);t.commit(q,out);check(Units::calls==0);
}
void readback_and_lost_reply() {
    for(int fault=0;fault<4;++fault) {
        Scene s;auto r=s.prepare();wire::Reply out;
        if(fault==0) Units::hook=[](df::unit *){throw std::runtime_error("native failure");};
        if(fault==1) Units::hook=[](df::unit *){};
        if(fault==2) Units::hook=[&](df::unit *u){u->status.labors[0]=true;s.b.hist_figure_id++;};
        if(fault==3) Units::hook=[&](df::unit *u){u->status.labors[0]=true;s.carpenter.assigned_units.clear();};
        s.commit(r,out);check(out.accepted&&out.unresolved&&engine.unresolved());
        check(s.mine.assigned_units==std::vector<std::int32_t>({2,5}));
        Units::hook={};s.commit(r,out);check(Units::calls==1);
        auto q=s.prepared_request("second");check(PrepareAssignment(s.output,&q,&out)==CR_OK&&!out.accepted&&out.code==8);
    }
    Scene s;auto r=s.prepare();wire::Reply out;out.fail_effect=true;s.commit(r,out);
    check(!out.accepted&&Units::calls==1&&!engine.unresolved());
    out.fail_effect=false;r.mask=17;check(QueryAssignment(s.output,&r,&out)==CR_OK&&out.accepted&&out.effect_present);
    s.commit(r,out);check(Units::calls==1);
}
void refusals() {
    for(int f=0;f<11;++f) {
        Scene s;auto r=s.prepare();wire::Reply out;
        if(f==0) s.game.external_flag.bits.automatic_professions_disabled=true;
        if(f==1) World::paused=false;
        if(f==2) World::folder="other";
        if(f==3) World::site=8;
        if(f==4) s.a.hist_figure_id=103;
        if(f==5) s.a.adult=false;
        if(f==6) s.mine.name="replacement detail";
        if(f==7) s.mine.allowed_labors[1]=true;
        if(f==8) s.carpenter.assigned_units.push_back(5);
        if(f==9) s.world.units.active.push_back(&s.a);
        if(f==10) unsetenv("DFMCP_WORKFORCE_ALLOW_LABOR");
        s.commit(r,out);check(Units::calls==0&&s.mine.assigned_units==std::vector<std::int32_t>({5}));
        if(f==10)check(!out.accepted&&out.code==1);else check(out.accepted&&!out.unresolved);
    }
    for(int f=0;f<8;++f) {
        Scene s;wire::Request r;r.ids={2,5};wire::Reply out;
        if(f==0)r.token="bad";
        if(f==1)r.nonce="bad";
        if(f==2)r.minor=16;
        if(f==3)r.unknown=1;
        if(f==4)r.initialized=false;
        if(f==5)r.size=2049;
        if(f==6)r.ids={5,2};
        if(f==7)s.a.citizen=false;
        check(ObserveWorkforce(s.output,&r,&out)==CR_OK&&!out.accepted&&Units::calls==0);
    }
}
void bounded_batch_and_overlapping_details() {
    Scene s; std::vector<df::unit> units(32); s.world.units.active.clear(); s.mine.assigned_units.clear();
    wire::Request r; r.key="batch"; r.mask=95;
    for (std::size_t i=0;i<units.size();++i) {
        auto &u=units[i];u.id=static_cast<std::int32_t>(i);u.hist_figure_id=static_cast<std::int32_t>(100+i);
        s.world.units.active.push_back(&u);r.ids.push_back(static_cast<std::uint32_t>(i));
    }
    {CoreSuspender guard;const auto capture=engine.observe(r.ids,read_native);r.witness=capture.witness();}
    r.plan=wf::plan_digest({0,true},r.witness);r.prepare=wf::token_for(r.key,r.plan);
    wire::Reply out;check(PrepareAssignment(s.output,&r,&out)==CR_OK&&out.accepted);s.commit(r,out);
    check(out.accepted&&!out.unresolved&&Units::calls==32&&s.mine.assigned_units.size()==32);
    for(const auto &u:units)check(u.status.labors[0]);
    // Another selected detail also grants mining. Removing one membership must
    // preserve the effective permission supplied by that other detail.
    s.carpenter.allowed_labors[0]=true;s.carpenter.assigned_units={0};
    wire::Request remove;remove.key="remove";remove.mask=95;remove.desired=false;remove.ids={0};
    {CoreSuspender guard;const auto capture=engine.observe(remove.ids,read_native);remove.witness=capture.witness();}
    remove.plan=wf::plan_digest({0,false},remove.witness);remove.prepare=wf::token_for(remove.key,remove.plan);
    check(PrepareAssignment(s.output,&remove,&out)==CR_OK&&out.accepted);s.commit(remove,out);
    check(out.accepted&&!out.unresolved&&Units::calls==33&&units[0].status.labors[0]);
    check(s.mine.assigned_units.size()==31&&s.carpenter.assigned_units==std::vector<std::int32_t>({0}));
    s.world.units.active.resize(4097,nullptr);wire::Request read;read.ids={0};
    check(ObserveWorkforce(s.output,&read,&out)==CR_OK&&!out.accepted&&Units::calls==33);
}
void shape_matrix() {
    using Handler=command_result(*)(color_ostream &,const wire::Request *,wire::Reply *);
    const Handler functions[]={Handshake,ObserveWorkforce,PrepareAssignment,CommitAssignment,QueryAssignment,CancelAssignment};
    const unsigned masks[]={0,64,95,49,17,49};
    for(unsigned op=0;op<6;++op)for(unsigned mask=0;mask<128;++mask) {
        Scene s;auto r=s.prepare();r.mask=mask;if(mask&64)r.ids={2,5};else r.ids.clear();wire::Reply out;
        check(functions[op](s.output,&r,&out)==CR_OK);
        check(out.accepted==(mask==masks[op]));
        if(mask!=masks[op])check(Units::calls==0);
    }
}
}
int main(){lifecycle();readback_and_lost_reply();refusals();bounded_batch_and_overlapping_details();shape_matrix();
    std::cout<<"workforce native handler: "<<checks<<" assertions passed\n";}
