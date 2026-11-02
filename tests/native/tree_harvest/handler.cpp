#include "stubs.h"
#include "../../../bridge/dfhack-tree-harvest-v1_21/dfmcp_tree_harvest_v1_21.cpp"
#include <iostream>
#include <sstream>
#include <iomanip>
namespace {
std::size_t checks=0;std::vector<std::string> groups;std::string group;
#define CHECK(x) do {++checks;if(!(x))throw std::runtime_error("check failed: "+group+":"+std::to_string(__LINE__)+": " #x);}while(false)
df::world world;df::plant tree;df::plant_tree_info info;std::array<df::plant_tree_tile,9> body;
df::plant_tree_tile *row=body.data();std::unique_ptr<df::building_wagonst> wagon;
DFHack::color_ostream output;
using Rpc=command_result(*)(color_ostream&,const wire::Request*,wire::Reply*);
void reset() {
    engine=th::Engine(7);world.jobs.list.next=nullptr;world.items.all.clear();world.units.active.clear();world.plants.all.clear();world.buildings.all.clear();wagon.reset();
    mock::items.clear();mock::units.clear();mock::jobs.clear();mock::links.clear();mock::general_refs.clear();mock::contained.clear();mock::specific_refs.clear();mock::blocks.clear();mock::walk_groups.clear();
    mock::loaded=mock::mapped=mock::fortress=mock::paused=mock::initialized=true;
    mock::unknown=mock::reply_failure=mock::oversized_reply=mock::oversized_request=false;
    mock::mark_fault=mock::MarkFault::None;mock::on_plant_lookup={};mock::tick=12345;mock::year=0;mock::site=1;
    mock::size_x=mock::size_y=64;mock::size_z=8;mock::next_item=1000;mock::folder="region1";mock::df_version="test-df";mock::dfhack_version="test-dfhack";
    df::global::world=&world;df::global::plotinfo=&world;df::global::item_next_id=&mock::next_item;
    for(int y=0;y<2;++y)for(int x=0;x<2;++x)mock::blocks[{x,y,2}]=std::make_unique<df::map_block>();
    body={};body[4].whole=body[5].whole=1;row=body.data();info.body=&row;info.dim_x=info.dim_y=3;info.body_height=1;
    tree={};tree.pos={15,15,2};tree.material=3;tree.tree_info=&info;world.plants.all.push_back(&tree);
    Maps::getTileBlock(15,15,2)->tiletype[15][15]=df::tiletype::TreeWall;
    Maps::getTileBlock(16,15,2)->tiletype[0][15]=df::tiletype::TreeWall;
    mock::units.push_back(std::make_unique<df::unit>());auto *u=mock::units.back().get();u->id=252;u->pos={20,20,2};u->status.labors[df::unit_labor::CUTWOOD]=true;world.units.active.push_back(u);
    mock::walk_groups[{20,20,2}]=1;mock::walk_groups[{10,11,2}]=1;mock::walk_groups[{17,15,2}]=1;
    wagon=std::make_unique<df::building_wagonst>();wagon->id=0;world.buildings.all.push_back(wagon.get());
    mock::items.push_back(std::make_unique<df::item>());auto *axe=mock::items.back().get();axe->id=62;axe->type=df::item_type::WEAPON;axe->subtype=0;axe->flags.whole=1u<<14;world.items.all.push_back(axe);
    mock::general_refs.push_back(std::make_unique<df::general_ref_building_holderst>());mock::general_refs.back()->holder=wagon.get();axe->general_refs.push_back(mock::general_refs.back().get());
    mock::contained.push_back(std::make_unique<df::buildingitemst>());mock::contained.back()->item=axe;mock::contained.back()->use_mode=0;wagon->contained_items.push_back(mock::contained.back().get());
    mock::materials[{420,163}].flags.values={df::material_flags::WOOD};
    unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");setenv("DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21","1",1);setenv("DFMCP_TREE_HARVEST_ALLOW_DESIGNATE","1",1);setenv("DFMCP_TREE_HARVEST_TOKEN",std::string(32,'s').c_str(),1);
    mock::writer_calls=mock::game_access=mock::helper_calls=0;
}
wire::Reply call(Rpc method,const wire::Request &request={}) {wire::Reply reply;CHECK(method(output,&request,&reply)==CR_OK);return reply;}
wire::Request prepared() {
    wire::Request q;q.mask=7;const auto reply=call(ReadTreeHarvest,q);CHECK(reply.accepted);
    q.witness=dfmcp_snapshot::sha256(reply.observation);q.plan=th::plan_digest(selection(&q),q.witness);q.mask=63;
    const auto p=call(PrepareTreeHarvest,q);CHECK(p.accepted&&!p.replayed);const auto *record=engine.query(q.key,q.plan);CHECK(record&&record->phase==th::Phase::Prepared);q.token=record->token;return q;
}
const th::Record &record(const wire::Request&q) {const auto *r=engine.query(q.key,q.plan);CHECK(r);return *r;}
wire::Reply commit(wire::Request q) {q.mask=104;return call(CommitTreeHarvest,q);}
void ineligible() {wire::Request q;q.mask=7;const auto r=call(ReadTreeHarvest,q);CHECK(r.accepted);q.witness=dfmcp_snapshot::sha256(r.observation);q.plan=th::plan_digest(selection(&q),q.witness);q.mask=63;
    CHECK(!call(PrepareTreeHarvest,q).accepted);CHECK(mock::writer_calls==0);}
void manifest_gates() {
    std::unique_ptr<RPCService> service(plugin_rpcconnect(output));CHECK(service->methods.size()==7);
    for(const auto &m:service->methods)CHECK(m.second==0);
    CHECK(call(Handshake).accepted);wire::Request q;q.minor=20;CHECK(!call(Handshake,q).accepted);
    setenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL","1.0",1);CHECK(!call(Handshake).accepted);unsetenv("DFMCP_ADMITTED_BRIDGE_PROTOCOL");
    q={};q.mask=1;CHECK(!call(ReadTreeHarvest,q).accepted);q={};mock::unknown=true;CHECK(!call(Handshake,q).accepted);mock::unknown=false;
    const auto p=prepared();unsetenv("DFMCP_TREE_HARVEST_ALLOW_DESIGNATE");CHECK(!commit(p).accepted);CHECK(mock::writer_calls==0);
}
void positive() {
    auto q=prepared();CHECK(record(q).before.selection.root.x==15);CHECK(record(q).before.designation.x==16);
    const auto r=commit(q);CHECK(r.accepted);CHECK(record(q).phase==th::Phase::Placed);CHECK(mock::writer_calls==1);
    CHECK(Maps::getTileBlock(15,15,2)->designation[15][15].bits.dig==df::tile_dig_designation::No);
    CHECK(Maps::getTileBlock(16,15,2)->designation[0][15].bits.dig==df::tile_dig_designation::Default);
    CHECK(Maps::getTileBlock(16,15,2)->flags.bits.designated);
    CHECK(record(q).insertion&&record(q).insertion->matches(record(q).before));
    CHECK(commit(q).accepted);CHECK(mock::writer_calls==1);q.mask=104;CHECK(call(CancelTreeHarvest,q).accepted);CHECK(record(q).phase==th::Phase::Placed);
}
void target_approaches() {
    wire::Request q;q.mask=7;auto cap=engine.inspect(selection(&q),capture);CHECK(cap.eligible());
    mock::walk_groups.erase({17,15,2});mock::walk_groups[{14,15,2}]=1;
    CHECK(!engine.inspect(selection(&q),capture).eligible());ineligible();
    reset();mock::walk_groups[{10,11,2}]=2;mock::walk_groups[{14,15,2}]=2;
    cap=engine.inspect(selection(&q),capture);CHECK(cap.workers[0].eligible());CHECK(!cap.tools[0].offer());CHECK(!cap.eligible());
}
void visibility_geometry_jobs() {
    wire::Request q;q.mask=7;Maps::getTileBlock(14,14,2)->designation[14][14].bits.hidden=1;
    const auto cap=engine.inspect(selection(&q),capture);CHECK(!cap.complete&&cap.body.empty());ineligible();
    reset();info.dim_x=0;CHECK(!call(ReadTreeHarvest,q).accepted);CHECK(mock::writer_calls==0);
    reset();mock::jobs.push_back(std::make_unique<df::job>());auto *j=mock::jobs.back().get();j->id=1;j->job_type=df::job_type::FellTree;j->pos={16,15,2};
    mock::links.push_back(std::make_unique<df::job_list_link>());auto *l=mock::links.back().get();l->item=j;l->prev=&world.jobs.list;j->list_link=l;world.jobs.list.next=l;
    ineligible();
    reset();mock::contained.back()->use_mode=2;ineligible();
}
void actors_tools() {
    for(unsigned which=0;which<7;++which) {reset();auto *u=world.units.active[0];switch(which){
        case 0:u->status.labors[df::unit_labor::CUTWOOD]=false;break;case 1:u->civ_permits_carpentry=false;break;case 2:u->has_current_job=true;break;
        case 3:u->adult=false;break;case 4:u->sane=false;break;case 5:world.items.all[0]->flags.whole|=1u<<15;break;default:world.items.all[0]->specific_refs.push_back(nullptr);break;}
        ineligible();}
    reset();const auto q=prepared();world.items.all[0]->flags.whole|=1u<<19;CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Refused);CHECK(mock::writer_calls==0);
}
void stale_lifetime_replay() {
    auto q=prepared();const auto created=record(q).created_ms;const auto prepared_bytes=record(q).encode();q.mask=63;CHECK(call(PrepareTreeHarvest,q).replayed);CHECK(record(q).created_ms==created&&record(q).encode()==prepared_bytes);
    mock::tick++;CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Refused&&mock::writer_calls==0);
    reset();q=prepared();const auto &expired=engine.commit(q.key,q.plan,q.token,record(q).created_ms+60000,capture,[](const th::Capture&b){designate(b,std::string(32,'s'));},verify);
    CHECK(expired.phase==th::Phase::Refused&&expired.reason==th::Reason::Expired&&mock::writer_calls==0);
    reset();q=prepared();engine.change_source();CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Refused&&mock::writer_calls==0);
    reset();q=prepared();Maps::getTileBlock(13,13,2)->occupancy[13][13].bits.other=1;
    CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Refused&&mock::writer_calls==0);
}
void uncertainty_recovery() {
    for(auto fault:{mock::MarkFault::Before,mock::MarkFault::DigOnly,mock::MarkFault::BlockOnly}) {
        reset();auto q=prepared();mock::mark_fault=fault;CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Indeterminate);CHECK(mock::writer_calls==1);
        CHECK(commit(q).accepted);q.mask=104;CHECK(call(CancelTreeHarvest,q).accepted);CHECK(record(q).phase==th::Phase::Indeterminate&&mock::writer_calls==1);
        q.mask=40;CHECK(call(QueryTreeHarvest,q).accepted);CHECK(record(q).phase==th::Phase::Indeterminate&&mock::writer_calls==1);
    }
    reset();auto q=prepared();mock::reply_failure=true;CHECK(!commit(q).accepted);CHECK(record(q).phase==th::Phase::Placed&&mock::writer_calls==1);q.mask=40;CHECK(call(QueryTreeHarvest,q).accepted);CHECK(mock::writer_calls==1);
    reset();q=prepared();mock::on_plant_lookup=[] {setenv("DFMCP_TREE_HARVEST_TOKEN",std::string(32,'x').c_str(),1);};
    CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Indeterminate&&mock::writer_calls==0);
    reset();q=prepared();mock::mark_fault=mock::MarkFault::FalseAfter;CHECK(commit(q).accepted);CHECK(record(q).phase==th::Phase::Placed&&mock::writer_calls==1);
}
void supply() {
    world.plants.all.clear();Maps::getTileBlock(15,15,2)->tiletype[15][15]=df::tiletype::Floor;Maps::getTileBlock(16,15,2)->tiletype[0][15]=df::tiletype::Floor;
    for(int id:{1000,1001}) {mock::items.push_back(std::make_unique<df::item>());auto *item=mock::items.back().get();item->id=id;item->pos={12,id-988,2};world.items.all.push_back(item);}mock::next_item=1002;
}
void progress_sort() {
    auto q=prepared();CHECK(commit(q).accepted);q.mask=391;const auto pending=call(ReadTreeHarvestProgress,q);CHECK(pending.accepted);
    supply();const auto first=call(ReadTreeHarvestProgress,q);CHECK(first.accepted);std::reverse(world.items.all.begin(),world.items.all.end());const auto second=call(ReadTreeHarvestProgress,q);
    CHECK(second.accepted&&first.observation==second.observation);CHECK(mock::writer_calls==1);
    world.items.all.front()->flags.whole|=1u<<14;const auto changed=call(ReadTreeHarvestProgress,q);CHECK(changed.accepted&&changed.observation!=first.observation);
}
void run(const char*name,void(*f)()) {reset();group=name;f();groups.emplace_back(name);}
std::string hex(const std::string&s) {std::ostringstream out;for(unsigned char c:s)out<<std::hex<<std::setw(2)<<std::setfill('0')<<static_cast<unsigned>(c);return out.str();}
void vectors() {
    reset();auto q=prepared();std::map<std::string,std::string> v;v["capture"]=record(q).before.encode();v["plan"]=q.plan;v["token"]=q.token;v["prepared"]=record(q).encode();commit(q);v["designated"]=record(q).encode();q.mask=391;
    v["progress_wait"]=call(ReadTreeHarvestProgress,q).observation;supply();v["progress_supply"]=call(ReadTreeHarvestProgress,q).observation;
    reset();q=prepared();mock::mark_fault=mock::MarkFault::Before;commit(q);v["indeterminate"]=record(q).encode();
    reset();q=prepared();mock::tick++;commit(q);v["refused"]=record(q).encode();reset();q=prepared();q.mask=104;call(CancelTreeHarvest,q);v["cancelled"]=record(q).encode();
    std::cout<<"{";bool first=true;for(const auto &[k,value]:v){if(!first)std::cout<<",";first=false;std::cout<<"\""<<k<<"\":\""<<hex(value)<<"\"";}std::cout<<"}\n";
}
}
int main(int argc,char**argv) {
    try {if(argc==2&&std::string(argv[1])=="--vectors"){vectors();return 0;}
        run("rpc_closed_surface_and_gates",manifest_gates);run("designate_supported_southeast_once",positive);run("computed_target_approach_witnesses",target_approaches);
        run("tree_visibility_geometry_and_job_conflicts",visibility_geometry_jobs);run("worker_and_tool_offers_are_separate",actors_tools);
        run("stale_source_lifetime_and_replay",stale_lifetime_replay);run("lost_reply_partial_write_and_recovery",uncertainty_recovery);run("progress_root_predicate_and_sorted_supply",progress_sort);
        std::cout<<"{\"scenarios\":"<<groups.size()<<",\"assertions\":"<<checks<<",\"ignored\":0,\"groups\":[";for(std::size_t i=0;i<groups.size();++i){if(i)std::cout<<",";std::cout<<"\""<<groups[i]<<"\"";}std::cout<<"]}\n";return 0;
    }catch(const std::exception&e){std::cerr<<e.what()<<"\n";return 1;}
}
