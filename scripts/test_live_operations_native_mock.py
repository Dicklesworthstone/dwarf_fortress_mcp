#!/usr/bin/env python3
"""Compile the real producer with mock interfaces; this is not a DFHack build."""
import argparse, hashlib, json, pathlib, struct, subprocess, tempfile

MOCK = r'''
#pragma once
#include <cstdint>
#include <memory>
#include <string>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
#define ENUM_KEY_STR(kind,value) mock_key(#kind,static_cast<int>(value))
inline std::string mock_key(const char *kind,int value) { return std::string(kind)+"_"+std::to_string(value); }
namespace df {
struct coord { int32_t x=0,y=0,z=0; };
enum class building_type { Workshop=1,Stockpile=2 };
enum class item_type { Drink=1,Barrel=2,Wood=3 };
enum class job_type { BrewDrink=1 };
enum class job_role_type { Reagent=1 };
struct building { int32_t id=0,x1=0,y1=0,x2=0,y2=0,z=0,stage=1,maximum=3;
 building_type type=building_type::Workshop;
 building_type getType() {return type;} int32_t getBuildStage(){return stage;}
 int32_t getMaxBuildStage(){return maximum;} };
struct item { int32_t id=0,subtype=-1,material=0,material_index=1,stack=1; coord pos;
 item_type type=item_type::Drink;
 struct {struct {bool forbid=false,in_job=false,dump=false,removed=false,rotten=false,trader=false,
 on_ground=false,in_inventory=false,in_building=false;} bits;} flags;
 std::vector<void *> general_refs; item *container=nullptr; building *holder=nullptr;
 item_type getType(){return type;} int32_t getSubtype(){return subtype;}
 int32_t getMaterial(){return material;} int32_t getMaterialIndex(){return material_index;}
 int32_t getStackSize(){return stack;} };
struct unit {int32_t id=9;};
struct job_item_ref {df::item *item=nullptr; job_role_type role=job_role_type::Reagent; int32_t job_item_idx=0;};
struct job {int32_t id=7,completion_timer=-1; df::job_type job_type=df::job_type::BrewDrink;
 std::string reaction_name="BREW"; coord pos;
 struct {struct {bool suspend=false,repeat=false;} bits;} flags;
 std::vector<job_item_ref *> items; struct {std::vector<void *> elements;} job_items;
 std::vector<void *> general_refs; unit *worker=nullptr; building *holder=nullptr;};
struct job_list_link {df::job *item=nullptr;job_list_link *next=nullptr;};
struct world {struct {job_list_link list;} jobs; struct {std::vector<building *> all;} buildings;
 struct {std::vector<item *> all;} items;};
namespace global {inline df::world *world=nullptr;inline int32_t *job_next_id=nullptr,*building_next_id=nullptr,*item_next_id=nullptr;}
}
namespace DFHack {
struct color_ostream{};struct PluginCommand{};enum command_result{CR_OK,CR_FAILURE};
enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_OTHER};
struct VersionInfo {std::string getVersion(){return "df";}};
struct Core {std::shared_ptr<VersionInfo> vinfo=std::make_shared<VersionInfo>();bool loaded=true;
 static Core &getInstance(){static Core core;return core;}bool isWorldLoaded(){return loaded;}};
namespace Version {inline std::string dfhack_version(){return "dfhack";}}
namespace World {inline bool fortress=true,paused=true;inline uint32_t year=105,tick=3;inline int32_t site=1;
 inline std::string folder="region1";
 inline bool isFortressMode(){return fortress;} inline uint32_t ReadCurrentYear(){return year;}
 inline uint32_t ReadCurrentTick(){return tick;} inline int32_t GetCurrentSiteId(){return site;}
 inline bool ReadPauseState(){return paused;} inline std::string ReadWorldFolder(){return folder;}}
namespace Job {inline df::unit *getWorker(df::job *job){return job->worker;}
 inline df::building *getHolder(df::job *job){return job->holder;}}
namespace Items {inline df::item *getContainer(df::item *item){return item->container;}
 inline df::building *getHolderBuilding(df::item *item){return item->holder;}}
struct RPCService {std::vector<std::string> names;std::vector<int> flags;
 template<class F>void addFunction(const char *name,F,int flag){names.push_back(name);flags.push_back(flag);}};
}
namespace dfmcp::operations::v1_3 {
struct Request {std::string token=std::string(32,'t'),nonce=std::string(16,'n');
 uint32_t major=1,minor=3,jobs=4096,buildings=4096,items=32768,bytes=2*1024*1024;
 const std::string &bearer_token()const{return token;}const std::string &client_nonce()const{return nonce;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}
 uint32_t max_jobs()const{return jobs;}uint32_t max_buildings()const{return buildings;}
 uint32_t max_items()const{return items;}uint32_t max_bytes()const{return bytes;}};
struct Reply {bool accepted=false;uint32_t code=0,major=0,minor=0;uint64_t generation=0;
 std::string nonce,df,dfhack,payload; bool has_payload=false;
 void Clear(){*this=Reply();}void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){code=v;}
 void set_client_nonce(const std::string &v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}
 void set_protocol_minor(uint32_t v){minor=v;}void set_bridge_generation(uint64_t v){generation=v;}
 void set_df_version(const std::string &v){df=v;}void set_dfhack_version(const std::string &v){dfhack=v;}
 void set_observation(const std::string &v){payload=v;has_payload=true;}};
}
'''
DRIVER = r'''
#include <iostream>
#include <stdexcept>
#include "PRODUCER"
int checks=0;
void check(bool value) {++checks;if(!value)throw std::runtime_error("check "+std::to_string(checks));}
int main() {
 setenv("DFMCP_OPERATIONS_TOKEN",std::string(32,'t').c_str(),1);
 df::world world; int32_t nj=8,nb=22,ni=33; df::global::world=&world;
 df::global::job_next_id=&nj;df::global::building_next_id=&nb;df::global::item_next_id=&ni;
 df::building workshop,stockpile;workshop.id=20;workshop.x1=1;workshop.y1=2;workshop.x2=3;workshop.y2=4;workshop.z=5;
 stockpile.id=21;stockpile.type=df::building_type::Stockpile;stockpile.stage=2;stockpile.maximum=2;
 df::item drink,barrel,wood;drink.id=30;drink.stack=5;drink.container=&barrel;drink.flags.bits.in_job=true;
 barrel.id=31;barrel.type=df::item_type::Barrel;barrel.holder=&workshop;barrel.flags.bits.in_building=true;
 wood.id=32;wood.type=df::item_type::Wood;wood.flags.bits.on_ground=true;wood.pos={1,2,5};
 df::job job;job.holder=&workshop;job.pos={1,2,5};df::job_item_ref attachment;attachment.item=&drink;
 job.items={&attachment};job.job_items.elements={&job};
 df::job_list_link link;link.item=&job;world.jobs.list.next=&link;
 world.buildings.all={&stockpile,&workshop};world.items.all={&wood,&drink,&barrel};
 wire::Request request;wire::Reply reply;color_ostream output;
 auto read=[&](){ReadObservation(output,&request,&reply);};
 auto rejected=[&](){read();check(!reply.accepted);check(reply.code!=0);check(!reply.has_payload);check(reply.payload.empty());};
 Handshake(output,&request,&reply);check(reply.accepted);check(!reply.has_payload);check(reply.minor==3);
 read();check(reply.accepted);check(reply.has_payload);check(reply.code==0);const auto golden=reply.payload;
 world.items.all={&drink,&barrel,&wood};world.buildings.all={&workshop,&stockpile};read();check(reply.payload==golden);
 request.token="wrong";rejected();request.token=std::string(32,'x');rejected();request.token=std::string(32,'t');
 request.minor=2;rejected();request.minor=3;request.nonce="x";rejected();request.nonce=std::string(16,'n');
 request.jobs=0;rejected();request.jobs=4096;request.buildings=1;rejected();request.buildings=4096;
 request.items=2;rejected();request.items=32768;request.bytes=1023;rejected();request.bytes=2*1024*1024;
 Core::getInstance().loaded=false;rejected();Core::getInstance().loaded=true;
 World::fortress=false;rejected();World::fortress=true;World::tick=403200;rejected();World::tick=3;
 world.items.all.push_back(nullptr);rejected();world.items.all.pop_back();
 world.buildings.all.push_back(&workshop);rejected();world.buildings.all.pop_back();
 ni=32;rejected();ni=33;nb=20;rejected();nb=22;nj=7;rejected();nj=8;
 workshop.stage=4;rejected();workshop.stage=1;workshop.x1=4;rejected();workshop.x1=1;
 drink.stack=-1;rejected();drink.stack=5;drink.material=-2;rejected();drink.material=0;
 df::item outside;outside.id=34;drink.container=&outside;rejected();drink.container=&barrel;
 barrel.container=&drink;rejected();barrel.container=nullptr;
 barrel.holder=&outside_holder; // substituted below with a declared building
 barrel.holder=&workshop;
 attachment.item=&outside;rejected();attachment.item=&drink;
 attachment.job_item_idx=1;rejected();attachment.job_item_idx=0;
 job.items.push_back(&attachment);rejected();job.items.pop_back();
 link.next=&link;request.jobs=3;rejected();link.next=nullptr;request.jobs=4096;
 job.reaction_name=std::string("a\0b",3);rejected();job.reaction_name="BREW";
 job.reaction_name=std::string("\xc0\x80",2);rejected();job.reaction_name="BREW";
 const auto saved=generation;plugin_onstatechange(output,SC_WORLD_UNLOADED);check(generation==saved+1);
 read();check(reply.accepted);check(reply.payload==golden);check(reply.generation==saved+1);
 auto *service=plugin_rpcconnect(output);check(service->names==std::vector<std::string>({"Handshake","ReadObservation"}));
 check(service->flags==std::vector<int>({0,0}));delete service;
 // Every flag occupies its own declared bit; no native layout is serialized.
 drink.flags.bits.forbid=true;drink.flags.bits.dump=true;drink.flags.bits.removed=true;
 drink.flags.bits.rotten=true;drink.flags.bits.trader=true;drink.flags.bits.on_ground=true;
 drink.flags.bits.in_inventory=true;drink.flags.bits.in_building=true;read();check(reply.accepted);
 // A roster that fits count limits but not bytes refuses atomically.
 std::vector<df::item> many(100);world.items.all.clear();
 for(int i=0;i<100;++i){many[i].id=i;world.items.all.push_back(&many[i]);}
 ni=100;world.jobs.list.next=nullptr;request.bytes=1024;rejected();request.bytes=2*1024*1024;
 world.items.all.clear();world.buildings.all.clear();read();check(reply.accepted);
 std::cout<<"{\"checks\":"<<checks<<",\"hex\":\"";
 const char *hex="0123456789abcdef";for(unsigned char c:golden)std::cout<<hex[c>>4]<<hex[c&15];
 std::cout<<"\"}\n";
}
'''.replace('barrel.holder=&outside_holder; // substituted below with a declared building',
'''df::building outside_holder;outside_holder.id=23;barrel.holder=&outside_holder;rejected();''')

def golden_frame():
    u=lambda n:struct.pack('>I',n)
    i=lambda n:struct.pack('>i',n)
    t=lambda s:struct.pack('>H',len(s.encode()))+s.encode()
    ref=lambda n:b'\0' if n is None else b'\1'+u(n)
    jobs=b'DFMJ1200'+u(105)+u(3)+b'\1'+i(1)+u(8)+t('region1')+u(1)
    jobs+=u(7)+i(1)+t('job_type_1')+t('BREW')+b'\0\0'+i(1)+i(2)+i(5)+ref(None)+ref(20)+i(-1)+u(1)+u(1)
    out=b'DFMO1300'+u(len(jobs))+jobs+u(22)+u(33)+u(2)
    for ident,typ,coords,stage,max_stage in [(20,1,[1,2,3,4,5],1,3),(21,2,[0,0,0,0,0],2,2)]:
        out+=u(ident)+i(typ)+t(f'building_type_{typ}')+b''.join(i(n) for n in coords)+i(stage)+i(max_stage)
    out+=u(3)
    for ident,typ,stack,coords,flags,container,holder in [(30,1,5,[0,0,0],2,31,None),(31,2,1,[0,0,0],256,None,20),(32,3,1,[1,2,5],64,None,None)]:
        out+=u(ident)+i(typ)+t(f'item_type_{typ}')+i(-1)+i(0)+i(1)+u(stack)+b''.join(i(n) for n in coords)+u(flags)+ref(container)+ref(holder)
    return out+u(1)+u(7)+u(30)+i(1)+i(0)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',default='g++');parser.add_argument('--output');args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1]
    source=root/'bridge/dfhack-operations-v1_3/dfmcp_operations_v1_3.cpp'
    with tempfile.TemporaryDirectory(prefix='dfmcp-operations-mock-') as tmp:
        tmp=pathlib.Path(tmp);(tmp/'mock.hpp').write_text(MOCK)
        for header in ['Core.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','MiscUtils.h','DfmcpOperationsV1_3.pb.h',
            'modules/Job.h','modules/Items.h','modules/World.h','df/building.h','df/building_type.h','df/global_objects.h',
            'df/item.h','df/item_type.h','df/job.h','df/job_item_ref.h','df/job_list_link.h','df/unit.h','df/world.h']:
            path=tmp/header;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('#include "mock.hpp"\n')
        (tmp/'test.cpp').write_text(DRIVER.replace('PRODUCER',str(source)))
        flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic']
        subprocess.run([args.compiler,*flags,'-I',str(tmp),str(tmp/'test.cpp'),'-o',str(tmp/'test')],check=True,timeout=60)
        run=subprocess.run([str(tmp/'test')],capture_output=True,text=True,check=True,timeout=15)
        result=json.loads(run.stdout);expected=golden_frame();assert result['hex']==expected.hex()
        golden=root/'crates/dfmcp-adapter/tests/fixtures/operations_v1_3.hex'
        if golden.exists(): assert golden.read_text().strip()==expected.hex()
        result={**result,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'compiler':args.compiler,
            'flags':flags,'golden_bytes':len(expected),'evidence':'mock_interfaces_only_not_native_dfhack_qualification'}
        if args.output:pathlib.Path(args.output).write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result,indent=2))
if __name__=='__main__':main()
