#include "stubs.h"
#include "../../../bridge/dfhack-dig-v1_16/dfmcp_dig_v1_16.cpp"
#include <functional>
#include <iostream>
std::size_t checks=0;
#define CHECK(x) do{++checks;if(!(x))throw std::runtime_error(std::string("line ")+std::to_string(__LINE__)+": " #x);}while(false)
DFHack::color_ostream output;df::world world;
void reset(){
    mock::blocks.clear();CHECK(mock::events_alive==0);mock::event_constructor_calls=mock::fail_constructor_at=0;
    mock::reserve_failure=mock::partial_failure=mock::reply_failure=mock::unknown=mock::missing=false;
    mock::initialized=mock::loaded=mock::mapped=mock::fortress=mock::paused=true;
    mock::year=0;mock::tick=12345;mock::site=1;mock::folder="region1";world.jobs.list.next=nullptr;df::global::world=&world;
    for(int z=1;z<=3;++z)for(int y=0;y<2;++y)for(int x=0;x<2;++x)mock::blocks[{x,y,z}]=std::make_unique<df::map_block>();
    engine.reset();setenv("DFMCP_ALLOW_UNADMITTED_DIG_V1_16","1",1);setenv("DFMCP_DIG_TOKEN",std::string(32,'s').c_str(),1);
    setenv("DFMCP_DIG_ALLOW_DESIGNATE","1",1);
}
wire::Request prepared(bool hidden=false){
    wire::Request q;q.mask=31;wire::Reply reply;CHECK(ReadDesignation(output,&q,&reply)==CR_OK);CHECK(reply.accepted);
    q.mask=511;q.hidden=hidden;q.witness=dfmcp_snapshot::sha256(reply.observation);q.plan=dg::plan_digest(region(&q),hidden,q.witness);
    CHECK(PrepareDesignation(output,&q,&reply)==CR_OK);CHECK(reply.accepted);CHECK(!reply.replayed);
    const auto*r=engine.query(q.key,q.plan);CHECK(r);q.token=r->token;return q;
}
wire::Reply commit(wire::Request q){q.mask=832;wire::Reply r;CHECK(CommitDesignation(output,&q,&r)==CR_OK);return r;}
df::map_block&target(){return *mock::blocks.at({0,0,2});}
void refused_prepare(bool hidden=false){
    wire::Request q;q.mask=31;wire::Reply r;CHECK(ReadDesignation(output,&q,&r)==CR_OK);CHECK(r.accepted);
    q.mask=511;q.hidden=hidden;q.witness=dfmcp_snapshot::sha256(r.observation);q.plan=dg::plan_digest(region(&q),hidden,q.witness);
    CHECK(PrepareDesignation(output,&q,&r)==CR_OK);CHECK(!r.accepted);CHECK(r.code==4);CHECK(mock::designated_tiles()==0);
}
void gates_and_shapes(){
    reset();wire::Request q;wire::Reply r;
    for(unsigned mask=0;mask<1024;++mask){q.mask=mask;CHECK(Handshake(output,&q,&r)==CR_OK);CHECK(r.accepted==(mask==0));
        CHECK(ReadDesignation(output,&q,&r)==CR_OK);CHECK(r.accepted==(mask==31));}
    q.mask=0;mock::unknown=true;Handshake(output,&q,&r);CHECK(!r.accepted&&r.code==3);mock::unknown=false;
    mock::initialized=false;Handshake(output,&q,&r);CHECK(!r.accepted);mock::initialized=true;
    q.minor=13;Handshake(output,&q,&r);CHECK(!r.accepted&&r.code==2);q.minor=16;
    q.bearer[0]='x';Handshake(output,&q,&r);CHECK(!r.accepted&&r.code==1);q.bearer=std::string(32,'s');
    for(auto n:{0u,15u,65u,10000u}){q.nonce=std::string(n,'n');Handshake(output,&q,&r);CHECK(!r.accepted);CHECK(r.nonce.size()<=64);}q.nonce=std::string(16,'n');
    unsetenv("DFMCP_ALLOW_UNADMITTED_DIG_V1_16");Handshake(output,&q,&r);CHECK(!r.accepted&&r.code==1);
    reset();auto plan=prepared();unsetenv("DFMCP_DIG_ALLOW_DESIGNATE");auto c=commit(plan);CHECK(!c.accepted&&c.code==1);CHECK(mock::designated_tiles()==0);
    plan.mask=320;QueryDesignation(output,&plan,&r);CHECK(r.accepted&&!r.effect.empty());plan.mask=31;ReadDesignation(output,&plan,&r);CHECK(r.accepted);
}
void visible_policy_and_redaction(){
    for(int kind=0;kind<13;++kind){reset();auto&b=target();auto&d=b.designation[15][15];auto&o=b.occupancy[15][15];
        switch(kind){case 0:mock::paused=false;break;case 1:d.bits.hidden=1;break;case 2:b.tiletype[15][15]=df::tiletype::Floor;break;
        case 3:b.tiletype[15][15]=df::tiletype::ConstructedWall;break;case 4:d.bits.dig=df::tile_dig_designation::Channel;break;
        case 5:d.bits.smooth=1;break;case 6:o.bits.building=1;break;case 7:o.bits.unit=1;break;case 8:o.bits.unit_grounded=1;break;
        case 9:o.bits.item=1;break;case 10:d.bits.water_table=1;break;case 11:d.bits.feature_local=1;break;case 12:d.bits.feature_global=1;break;}
        refused_prepare(true);
    }
    for(int kind=0;kind<5;++kind){reset();auto&b=*mock::blocks.at({0,0,1});
        switch(kind){case 0:b.temperature_1[14][14]=10081;break;case 1:b.temperature_2[14][14]=10081;break;
        case 2:b.designation[14][14].bits.flow_size=1;break;case 3:b.designation[14][14].bits.water_table=1;break;
        case 4:mock::missing=true;mock::missing_block={0,0,1};break;}refused_prepare(true);}
    reset();auto&b=*mock::blocks.at({0,0,1});b.designation[14][14].bits.hidden=1;
    refused_prepare();wire::Request q;q.mask=31;wire::Reply a,c;ReadDesignation(output,&q,&a);CHECK(a.accepted);
    b.tiletype[14][14]=df::tiletype::Bad;b.temperature_1[14][14]=65535;b.occupancy[14][14].whole=UINT32_MAX;
    b.designation[14][14].bits.water_table=1;b.designation[14][14].bits.flow_size=7;
    ReadDesignation(output,&q,&c);CHECK(c.accepted&&a.observation==c.observation);
    auto plan=prepared(true);auto result=commit(plan);CHECK(result.accepted);CHECK(engine.query(plan.key,plan.plan)->state==dg::State::Designated);
}
void complete_job_and_event_validation(){
    reset();df::job a{1,{15,15,2}},b{2,{50,50,5}};df::job_list_link la{&a,nullptr},lb{&b,nullptr};la.next=&lb;world.jobs.list.next=&la;
    refused_prepare();lb.item=nullptr;wire::Request q;q.mask=31;wire::Reply r;ReadDesignation(output,&q,&r);CHECK(!r.accepted&&r.code==5);
    lb.item=&a;ReadDesignation(output,&q,&r);CHECK(!r.accepted&&r.code==5);lb.item=&b;lb.next=&la;
    ReadDesignation(output,&q,&r);CHECK(!r.accepted&&r.code==5);world.jobs.list.next=nullptr;
    reset();std::vector<df::job>jobs(MAX_JOBS+1);std::vector<df::job_list_link>links(MAX_JOBS+1);
    for(std::size_t i=0;i<links.size();++i){jobs[i].id=static_cast<int>(i);jobs[i].pos={50,50,5};links[i].item=&jobs[i];links[i].next=i+1<links.size()?&links[i+1]:nullptr;}
    world.jobs.list.next=links.data();ReadDesignation(output,&q,&r);CHECK(!r.accepted&&r.code==5);
    links[MAX_JOBS-1].next=nullptr;ReadDesignation(output,&q,&r);CHECK(r.accepted);world.jobs.list.next=nullptr;
    reset();target().block_events.push_back(new df::block_square_event_designation_priorityst());
    target().block_events.push_back(new df::block_square_event_designation_priorityst());ReadDesignation(output,&q,&r);CHECK(!r.accepted&&r.code==5);
    reset();target().block_events.push_back(nullptr);ReadDesignation(output,&q,&r);CHECK(!r.accepted&&r.code==5);target().block_events.pop_back();
}
void real_writer_preserves_non_targets(){
    for(auto material:{df::tiletype::StoneWall,df::tiletype::SoilWall,df::tiletype::MineralWall}){
        reset();for(unsigned x=15;x<=16;++x)for(unsigned y=15;y<=16;++y)Maps::getTileBlock(x,y,2)->tiletype[x&15][y&15]=material;
        auto&b=target();auto*p=new df::block_square_event_designation_priorityst();p->priority[14][14]=2000;b.block_events.push_back(p);
        b.designation[14][14].bits.dig=df::tile_dig_designation::Channel;b.flags.bits.other=11;
        df::job outside{99,{14,14,2}};df::job_list_link link{&outside,nullptr};world.jobs.list.next=&link;
        auto q=prepared();auto c=commit(q);CHECK(c.accepted);const auto*r=engine.query(q.key,q.plan);CHECK(r&&r->state==dg::State::Designated);
        CHECK(r->designated_count==4);CHECK(mock::designated_tiles()==4);CHECK(mock::events_alive==4);
        CHECK(world.jobs.list.next==&link&&outside.id==99);CHECK(b.designation[14][14].bits.dig==df::tile_dig_designation::Channel);
        CHECK(p->priority[14][14]==2000);CHECK(b.flags.bits.other==11);
        for(unsigned x=15;x<=16;++x)for(unsigned y=15;y<=16;++y){auto*block=Maps::getTileBlock(x,y,2);CHECK(block->dsgn_check_cooldown==0);CHECK(block->flags.bits.designated);
            CHECK(priority_event(block)->priority[x&15][y&15]==4000);CHECK(block->tiletype[x&15][y&15]==material);}
        const auto old=r->encode();CHECK(commit(q).effect==old);CHECK(mock::events_alive==4);world.jobs.list.next=nullptr;
    }
}
void allocation_partial_and_lost_reply(){
    for(int fault=0;fault<3;++fault){reset();auto q=prepared();
        if(fault==0)mock::reserve_failure=true;
        if(fault==1)mock::fail_constructor_at=3;
        if(fault==2)mock::partial_failure=true;
        auto reply=commit(q);mock::reserve_failure=false;mock::fail_constructor_at=0;
        const auto*r=engine.query(q.key,q.plan);CHECK(r&&r->state==dg::State::Unknown);CHECK(reply.accepted);CHECK(!r->after_known);
        if(fault<2){CHECK(mock::events_alive==0);CHECK(mock::designated_tiles()==0);}else CHECK(mock::designated_tiles()==1);
        auto count=mock::designated_tiles();CHECK(commit(q).effect==r->encode());CHECK(mock::designated_tiles()==count);
        q.mask=832;wire::Reply cancelled;CancelDesignation(output,&q,&cancelled);CHECK(cancelled.accepted);CHECK(engine.query(q.key,q.plan)->state==dg::State::Unknown);
    }
    reset();auto q=prepared();mock::reply_failure=true;auto reply=commit(q);CHECK(!reply.accepted&&reply.code==5);CHECK(reply.effect.empty());
    CHECK(engine.query(q.key,q.plan)->state==dg::State::Designated);CHECK(mock::designated_tiles()==4);
    q.mask=320;QueryDesignation(output,&q,&reply);CHECK(reply.accepted&&!reply.effect.empty());CHECK(mock::designated_tiles()==4);
}
void cancellation_and_lifecycle(){
    reset();auto q=prepared();q.mask=832;wire::Reply r;CancelDesignation(output,&q,&r);CHECK(r.accepted);
    CHECK(engine.query(q.key,q.plan)->reason==dg::Reason::Cancelled);CHECK(commit(q).effect==r.effect);CHECK(mock::designated_tiles()==0);
    reset();q=prepared();plugin_onstatechange(output,SC_UNPAUSED);r=commit(q);CHECK(r.accepted);CHECK(engine.query(q.key,q.plan)->state==dg::State::Refused);
    auto old_generation=engine.generation();plugin_onstatechange(output,SC_MAP_LOADED);CHECK(engine.generation()==old_generation+1);
    q.mask=320;QueryDesignation(output,&q,&r);CHECK(r.accepted&&r.effect.empty());
    std::vector<PluginCommand>commands;CHECK(plugin_init(output,commands)==CR_OK);CHECK(commands.empty());
    std::unique_ptr<RPCService>service(plugin_rpcconnect(output));CHECK(service->methods.size()==6);
    const char*names[]={"Handshake","ReadDesignation","PrepareDesignation","CommitDesignation","QueryDesignation","CancelDesignation"};
    for(unsigned i=0;i<6;++i){CHECK(service->methods[i].first==names[i]);CHECK(service->methods[i].second==0);}
    CHECK(plugin_shutdown(output)==CR_OK);
}
int main(){try{gates_and_shapes();visible_policy_and_redaction();complete_job_and_event_validation();real_writer_preserves_non_targets();
    allocation_partial_and_lost_reply();cancellation_and_lifecycle();mock::blocks.clear();CHECK(mock::events_alive==0);
    std::cout<<"{\"groups\":6,\"assertions\":"<<checks<<"}\n";return 0;
}catch(const std::exception&e){std::cerr<<e.what()<<'\n';return 1;}}
