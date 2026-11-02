#include "stubs.h"
#include "../../../bridge/dfhack-manager-v1_22/dfmcp_manager_v1_22.cpp"
#include <iostream>
namespace {
unsigned assertions=0;
#define CHECK(x) do{++assertions;if(!(x))throw std::runtime_error(std::string("check failed: ")+ #x +" line "+std::to_string(__LINE__));}while(false)
struct Fixture {
    df::world world;df::plotinfost plot;df::gamest game;
    df::historical_entity civ,group;df::entity_position other,leader,manager;
    df::entity_position_assignment unrelated,second,lead,appointment;
    df::historical_figure figure,other_figure;df::unit unit,other_unit;
    wire::Request request;DFHack::color_ostream output;
    Fixture() {
        mock::loaded=mock::mapped=mock::fortress=mock::paused=mock::initialized=true;mock::unknown=false;mock::getter_failure=false;
        mock::world_reads=mock::delay_millis=0;mock::read_hook={};mock::year=30;mock::tick=16800;mock::site=38;mock::next_event=1992;
        mock::focus={"dwarfmode/Default"};engine=ma::Engine(7);
        setenv("DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22","1",1);setenv("DFMCP_MANAGER_ALLOW_APPOINT","1",1);setenv("DFMCP_MANAGER_TOKEN",std::string(32,'s').c_str(),1);unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
        df::global::world=&world;df::global::plotinfo=&plot;df::global::game=&game;
        civ.id=12;civ.positions.next_position_id=24;civ.positions.next_assignment_id=7;
        group.id=35;group.positions.next_position_id=1234;group.positions.next_assignment_id=777;world.entities.all={&civ,&group};
        other.id=4;other.code="OTHER";leader.id=8;leader.code="EXPEDITION_LEADER";manager.id=1000;manager.code="MANAGER";
        group.positions.own={&other,&leader,&manager};
        unrelated.id=11;unrelated.position_id=4;unrelated.flags.data={0x80};unrelated.temp=42;
        second.id=22;second.position_id=4;lead.id=91;lead.position_id=8;lead.histfig=lead.histfig2=361;lead.position_vector_idx=1;
        appointment.id=600;appointment.position_id=1000;appointment.flags.data={1};group.positions.assignments={&unrelated,&second,&lead,&appointment};
        figure.id=361;figure.unit_id=256;other_figure.id=367;other_figure.unit_id=250;world.history.figures={&figure,&other_figure};
        unit.id=256;unit.hist_figure_id=361;unit.civ_id=12;unit.nemesis.figure=&figure;
        other_unit.id=250;other_unit.hist_figure_id=367;other_unit.civ_id=12;other_unit.nemesis.figure=&other_figure;world.units.active={&other_unit,&unit};
        for(int entity_id:{12,35}){auto*l=new df::histfig_entity_link_memberst();l->entity_id=entity_id;l->entity_vector_idx=entity_id==12?0:1;l->link_strength=100;figure.entity_links.push_back(l);}
        auto*l=df::allocate<df::histfig_entity_link_positionst>();l->entity_id=35;l->entity_vector_idx=1;l->link_strength=100;l->assignment_id=91;l->assignment_vector_idx=2;l->start_year=30;figure.entity_links.push_back(l);
        auto*old=new df::history_event();old->id=26;old->year=1;world.history.events.push_back(old);
        auto*recent=df::allocate<df::history_event_add_hf_entity_linkst>();recent->id=1991;recent->year=30;recent->seconds=16800;recent->civ=35;recent->histfig=361;recent->link_type=df::histfig_entity_link_type::POSITION;recent->position_id=8;world.history.events.push_back(recent);
        mock::allocations=mock::fail_allocation=0;
    }
    ~Fixture(){for(auto*f:world.history.figures)for(auto*l:f->entity_links)delete l;for(auto*e:world.history.events)delete e;mock::read_hook={};}
    wire::Reply read(){request.mask=3;wire::Reply reply;ReadManager(output,&request,&reply);return reply;}
    const ma::Record &prepare(){auto reply=read();CHECK(reply.accepted);request.mask=31;request.witness=dfmcp_snapshot::sha256(reply.observation);request.plan=ma::plan_digest({request.uid,request.hfid},request.witness);
        PrepareManager(output,&request,&reply);CHECK(reply.accepted);auto*r=engine.query(request.key,request.plan);CHECK(r && r->phase==ma::Phase::Prepared);request.token=r->token;return *r;}
    const ma::Record &commit(){request.mask=52;wire::Reply reply;CommitManager(output,&request,&reply);CHECK(reply.accepted);const auto*r=engine.query(request.key,request.plan);CHECK(r);return *r;}
};
void closed_rpc_and_authority_gates(){
    Fixture f;std::unique_ptr<DFHack::RPCService>s(plugin_rpcconnect(f.output));CHECK(s->names==std::vector<std::string>({"Handshake","ReadManager","PrepareManager","CommitManager","QueryManager","CancelManager"}));
    for(int mode=0;mode<5;++mode){f.request=wire::Request();f.request.mask=3;
        if(mode==0)f.request.bearer="bad";
        if(mode==1)f.request.minor=21;
        if(mode==2)mock::unknown=true;
        if(mode==3)setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL","1.0",1);
        if(mode==4)unsetenv("DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22");
        wire::Reply r;ReadManager(f.output,&f.request,&r);CHECK(!r.accepted);CHECK(mock::world_reads==0);CHECK(f.world.history.events.size()==2);
        mock::unknown=false;unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");setenv("DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22","1",1);
    }
}
void full_transaction_distinct_ids_and_indices(){
    Fixture f;const auto before=f.prepare().before;CHECK(before.eligible());CHECK(before.manager_position==1000 && before.manager_index==2 && before.group.index==1 && before.target()->id==600);
    const auto&r=f.commit();CHECK(r.phase==ma::Phase::Placed);CHECK(r.after && r.insertion && r.insertion->matches(before));CHECK(r.after->encode()==before.expected_after().encode());
    CHECK(f.appointment.histfig==361 && f.appointment.histfig2==361 && f.appointment.position_vector_idx==2);
    auto*l=strict_virtual_cast<df::histfig_entity_link_positionst>(f.figure.entity_links.back());CHECK(l && l->entity_id==35 && l->entity_vector_idx==1 && l->assignment_id==600 && l->assignment_vector_idx==3 && l->start_year==30 && l->link_strength==100);
    auto*e=strict_virtual_cast<df::history_event_add_hf_entity_linkst>(f.world.history.events.back());CHECK(e && e->id==1992 && e->year==30 && e->seconds==16800 && e->civ==35 && e->histfig==361 && e->position_id==1000 && e->appointer_hfid==-1 && e->promise_to_hfid==-1);
    CHECK(f.world.history.events.size()==3 && mock::next_event==1993 && mock::tick==16800 && mock::paused);
    CHECK(assignment(f.unrelated).encode()==before.group.assignments[0].encode());CHECK(assignment(f.lead).encode()==before.group.assignments[2].encode());CHECK(f.group.id==35);
    CHECK(DFHack::Units::getUnitByNobleRole("MANAGER")==&f.unit);std::vector<DFHack::Units::NoblePosition>roles;CHECK(DFHack::Units::getNoblePositions(&roles,&f.unit));CHECK(roles.size()==2);CHECK(mock::allocations==2);
}
void stale_identity_source_epoch_refuse(){
    for(unsigned mode=0;mode<6;++mode){Fixture f;f.prepare();
        if(mode==0)f.appointment.histfig=100;
        if(mode==1)f.unit.hist_figure_id=999;
        if(mode==2)f.figure.unit_id=999;
        if(mode==3)plugin_onstatechange(f.output,SC_WORLD_LOADED);
        if(mode==4)++mock::tick;
        if(mode==5)f.unrelated.temp=43;
        const auto&r=f.commit();CHECK(r.phase==ma::Phase::Refused);CHECK(f.world.history.events.size()==2 && mock::next_event==1992 && mock::allocations==0);
    }
}
void occupied_nonleader_and_membership_refuse(){
    for(unsigned mode=0;mode<5;++mode){Fixture f;
        if(mode==0)f.appointment.histfig=f.appointment.histfig2=361;
        if(mode==1)f.lead.histfig=-1;
        if(mode==2)f.unit.citizen=false;
        if(mode==3)f.figure.entity_links[0]->link_strength=0;
        if(mode==4)f.unit.nemesis.figure=&f.other_figure;
        auto r=f.read();if(r.accepted){f.request.mask=31;f.request.witness=dfmcp_snapshot::sha256(r.observation);f.request.plan=ma::plan_digest({256,361},f.request.witness);PrepareManager(f.output,&f.request,&r);}
        CHECK(!r.accepted);CHECK(f.world.history.events.size()==2 && mock::allocations==0);
    }
}
void active_and_pending_ui_refuse(){
    for(unsigned mode=0;mode<6;++mode){Fixture f;f.prepare();auto&a=f.game.main_interface.info.administrators;f.game.main_interface.info.open=true;
        if(mode==0)mock::focus={"dwarfmode/Info/ADMINISTRATORS/Default"};
        if(mode==1)a.choosing_candidate=true;
        if(mode==2)a.assigning_symbol=true;
        if(mode==3)a.spec_prof.push_back(&f.appointment);
        if(mode==4)a.spec_hfid.push_back(361);
        if(mode==5)a.spec_enid.push_back(35);
        const auto&r=f.commit();CHECK(r.phase==ma::Phase::Refused);CHECK(f.appointment.histfig==-1 && f.world.history.events.size()==2 && mock::allocations==0);
    }
    {Fixture f;auto&a=f.game.main_interface.info.administrators;a.spec_prof.push_back(&f.appointment);a.spec_hfid.push_back(361);a.spec_enid.push_back(35);a.choosing_candidate=true;
        CHECK(f.prepare().before.eligible());CHECK(f.appointment.histfig==-1 && f.world.history.events.size()==2);}
}
void allocation_and_budget_fail_before_world_writes(){
    for(unsigned fail:{1u,2u}){Fixture f;f.prepare();mock::fail_allocation=fail;const auto&r=f.commit();CHECK(r.phase==ma::Phase::Indeterminate);CHECK(f.appointment.histfig==-1 && f.figure.entity_links.size()==3 && f.world.history.events.size()==2 && mock::next_event==1992);}
    {Fixture f;f.prepare();f.request.mask=52;f.request.wall=1;mock::delay_millis=10;wire::Reply r;CommitManager(f.output,&f.request,&r);CHECK(f.appointment.histfig==-1 && f.world.history.events.size()==2 && mock::allocations==0);}
    {Fixture f;f.prepare();f.request.mask=52;f.request.wall=0;wire::Reply r;CommitManager(f.output,&f.request,&r);CHECK(!r.accepted && f.world.history.events.size()==2 && mock::allocations==0);}
}
void repeated_key_cancel_and_lost_reply_no_extra_event(){
    {Fixture f;const auto created=f.prepare().created_ms;wire::Reply r;PrepareManager(f.output,&f.request,&r);CHECK(r.accepted && r.replayed);CHECK(engine.query(f.request.key,f.request.plan)->created_ms==created);
        f.commit();const auto bytes=engine.query(f.request.key,f.request.plan)->encode();for(unsigned i=0;i<3;++i){f.commit();CHECK(engine.query(f.request.key,f.request.plan)->encode()==bytes);}
        f.request.mask=20;QueryManager(f.output,&f.request,&r);CHECK(r.accepted && f.world.history.events.size()==3 && mock::next_event==1993 && mock::allocations==2);}
    {Fixture f;f.prepare();f.request.mask=52;wire::Reply r;CancelManager(f.output,&f.request,&r);CHECK(r.accepted);CHECK(f.commit().phase==ma::Phase::Cancelled);CHECK(f.world.history.events.size()==2 && mock::allocations==0);}
}
void failed_readback_retains_uncertainty_and_recovers_once(){
    Fixture f;f.prepare();mock::read_hook=[&]{if(f.appointment.histfig==361)mock::getter_failure=true;};
    CHECK(f.commit().phase==ma::Phase::Indeterminate);CHECK(f.appointment.histfig==361 && f.world.history.events.size()==3 && mock::next_event==1993);
    CHECK(f.commit().phase==ma::Phase::Indeterminate);CHECK(f.world.history.events.size()==3 && mock::allocations==2);
    mock::read_hook={};mock::getter_failure=false;f.request.mask=20;wire::Reply r;QueryManager(f.output,&f.request,&r);CHECK(r.accepted);CHECK(engine.query(f.request.key,f.request.plan)->phase==ma::Phase::Placed);
    CHECK(f.world.history.events.size()==3 && mock::next_event==1993 && mock::allocations==2);
}
void stale_history_flags_links_and_position_definition_refuse(){
    for(unsigned mode=0;mode<5;++mode){Fixture f;f.prepare();
        if(mode==0)f.world.history.events[0]->flags.data={1};
        if(mode==1)f.figure.entity_links[0]->entity_vector_idx=44;
        if(mode==2)f.manager.required_office=2;
        if(mode==3)f.appointment.flags.data={3};
        if(mode==4)++mock::next_event;
        CHECK(f.commit().phase==ma::Phase::Refused);CHECK(f.world.history.events.size()==2 && f.appointment.histfig==-1 && mock::allocations==0);
    }
}
void encoding_bounds_and_source_domains(){
    Fixture f;auto before=f.prepare().before;auto bad=before;bad.generation=0;bool refused=false;try{(void)bad.encode();}catch(const dfmcp_workshop::Failure&){refused=true;}CHECK(refused);
    bad=before;bad.links[0].type=99;refused=false;try{(void)bad.encode();}catch(const dfmcp_workshop::Failure&){refused=true;}CHECK(refused);
    CHECK(ma::plan_digest(before.selection,before.witness())!=ma::hash("dfmcp-tree-harvest-plan/1",before.selection.encode()+before.witness()));
}
void sdk_event_flag_storage_and_assignment_bytes_are_preserved(){
    Fixture f;f.appointment.flags.data={0x81};
    std::unique_ptr<df::history_event_add_hf_entity_linkst> fresh(df::allocate<df::history_event_add_hf_entity_linkst>());
    CHECK(fresh && flags(fresh->flags)==std::string(1,'\0'));
    CHECK(!flags(fresh->flags).empty());mock::allocations=0;
    const auto before=f.prepare().before;
    CHECK(before.appointment_event().flags==flags(fresh->flags));
    CHECK(f.commit().phase==ma::Phase::Placed);
    CHECK(f.appointment.flags.data==std::vector<unsigned char>({0x81}));
    CHECK(flags(f.world.history.events.back()->flags)==std::string(1,'\0'));
    CHECK(mock::allocations==2);
}
void inactive_temporary_or_nonordinary_assignment_refuses(){
    for(unsigned mode=0;mode<10;++mode){Fixture f;
        if(mode==0)f.appointment.flags.data.clear();
        if(mode==1)f.appointment.flags.data={0};
        if(mode==2)f.appointment.flags.data={3};
        if(mode==3)f.appointment.flags.data={5};
        if(mode==4)f.appointment.flags.data={9};
        if(mode==5)f.appointment.st_id=4;
        if(mode==6)f.appointment.ab_id=4;
        if(mode==7)f.appointment.vassal_of_entity_id=4;
        if(mode==8)f.appointment.vassal_of_position_profile_id=4;
        if(mode==9)f.appointment.temp=4;
        const auto r=f.read();CHECK(r.accepted);
        f.request.mask=31;f.request.witness=dfmcp_snapshot::sha256(r.observation);f.request.plan=ma::plan_digest({256,361},f.request.witness);
        wire::Reply reply;PrepareManager(f.output,&f.request,&reply);
        CHECK(!reply.accepted && f.appointment.histfig==-1 && f.world.history.events.size()==2 && mock::allocations==0);
    }
}
std::string hex(const std::string&s){static const char*d="0123456789abcdef";std::string out;for(unsigned char c:s){out+=d[c>>4];out+=d[c&15];}return out;}
} // namespace
int main(int argc,char**argv){
    try{
        if(argc==2 && std::string(argv[1])=="--vectors"){Fixture f;const auto&r=f.prepare();const auto capture=r.before.encode(),prepared=r.encode(),plan=r.plan,token=r.token;const auto assigned=f.commit().encode();std::cout<<"{\"capture\":\""<<hex(capture)<<"\",\"prepared\":\""<<hex(prepared)<<"\",\"plan\":\""<<hex(plan)<<"\",\"token\":\""<<hex(token)<<"\",\"assigned\":\""<<hex(assigned)<<"\"}\n";return 0;}
        closed_rpc_and_authority_gates();full_transaction_distinct_ids_and_indices();stale_identity_source_epoch_refuse();occupied_nonleader_and_membership_refuse();active_and_pending_ui_refuse();allocation_and_budget_fail_before_world_writes();repeated_key_cancel_and_lost_reply_no_extra_event();failed_readback_retains_uncertainty_and_recovers_once();stale_history_flags_links_and_position_definition_refuse();encoding_bounds_and_source_domains();
        sdk_event_flag_storage_and_assignment_bytes_are_preserved();inactive_temporary_or_nonordinary_assignment_refuses();
        std::cout<<"{\"scenarios\":12,\"assertions\":"<<assertions<<",\"ignored\":0,\"sdk_doubles_only\":true,\"live_game\":false}\n";return 0;
    }catch(const std::exception&e){std::cerr<<e.what()<<'\n';return 1;}
}
