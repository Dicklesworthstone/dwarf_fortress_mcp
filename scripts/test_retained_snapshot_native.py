#!/usr/bin/env python3
"""Compile actual paging producer with mock interfaces, not a real DFHack build."""
import argparse, hashlib, json, pathlib, struct, subprocess, tempfile

MOCK = r'''
#pragma once
#include <cstdint>
#include <memory>
#include <string>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
#define ENUM_KEY_STR(kind,value) (std::string(#kind)+"_"+std::to_string(static_cast<int>(value)))
namespace df {
struct coord {int32_t x=0,y=0,z=0;};
enum class building_type {Workshop=1}; enum class item_type {Bar=1};enum class job_type {Make=1};enum class job_role_type {Reagent=1};
struct building {int32_t id=0,x1=0,y1=0,x2=0,y2=0,z=0,stage=1,maximum=3;
 building_type getType(){return building_type::Workshop;}int32_t getBuildStage(){return stage;}int32_t getMaxBuildStage(){return maximum;}};
struct item {int32_t id=0,subtype=-1,material=0,material_index=1,stack=1;coord pos;
 struct {struct {bool forbid=false,in_job=false,dump=false,removed=false,rotten=false,trader=false,on_ground=false,in_inventory=false,in_building=false;}bits;}flags;
 std::vector<void*>general_refs;item*container=nullptr;building*holder=nullptr;
 item_type getType(){return item_type::Bar;}int32_t getSubtype(){return subtype;}int32_t getMaterial(){return material;}
 int32_t getMaterialIndex(){return material_index;}int32_t getStackSize(){return stack;}};
struct unit {int32_t id=9;};
struct job_item_ref {df::item*item=nullptr;job_role_type role=job_role_type::Reagent;int32_t job_item_idx=0;};
struct job {int32_t id=7,completion_timer=-1;df::job_type job_type=df::job_type::Make;std::string reaction_name="MAKE";coord pos;
 struct{struct{bool suspend=false,repeat=false;}bits;}flags;std::vector<job_item_ref*>items;
 struct{std::vector<void*>elements;}job_items;std::vector<void*>general_refs;unit*worker=nullptr;building*holder=nullptr;};
struct job_list_link {df::job*item=nullptr;job_list_link*next=nullptr;};
struct world {struct{job_list_link list;}jobs;struct{std::vector<building*>all;}buildings;struct{std::vector<item*>all;}items;};
namespace global {inline df::world*world=nullptr;inline int32_t*job_next_id=nullptr,*building_next_id=nullptr,*item_next_id=nullptr;}
}
namespace DFHack {
struct color_ostream{};struct PluginCommand{};enum command_result{CR_OK,CR_FAILURE};enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_OTHER};
struct VersionInfo{std::string getVersion(){return "df";}};struct Core{std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();bool loaded=true;
 static Core&getInstance(){static Core v;return v;}bool isWorldLoaded(){return loaded;}};
namespace Version{inline std::string dfhack_version(){return "dfhack";}}
namespace World{inline uint32_t year=105,tick=3;inline int32_t site=1;inline bool fortress=true,paused=true;inline unsigned reads=0;
 inline bool isFortressMode(){return fortress;}inline uint32_t ReadCurrentYear(){return year;}
 inline uint32_t ReadCurrentTick(){++reads;return tick;}inline int32_t GetCurrentSiteId(){return site;}
 inline bool ReadPauseState(){return paused;}inline std::string ReadWorldFolder(){return "region1";}}
namespace Job{inline df::unit*getWorker(df::job*v){return v->worker;}inline df::building*getHolder(df::job*v){return v->holder;}}
namespace Items{inline df::item*getContainer(df::item*v){return v->container;}inline df::building*getHolderBuilding(df::item*v){return v->holder;}}
struct RPCService{std::vector<std::string>names;std::vector<int>flags;template<class F>void addFunction(const char*n,F,int f){names.push_back(n);flags.push_back(f);}};
}
namespace dfmcp::operations::v1_4 {
struct Request{std::string token=std::string(32,'t'),nonce=std::string(16,'n'),snapshot;
 uint32_t major=1,minor=4,jobs=4096,buildings=4096,items=65536,bytes=16*1024*1024,page=16384;uint64_t start=0;bool releasing=false;
 const std::string&bearer_token()const{return token;}const std::string&client_nonce()const{return nonce;}const std::string&snapshot_token()const{return snapshot;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}uint32_t max_jobs()const{return jobs;}
 uint32_t max_buildings()const{return buildings;}uint32_t max_items()const{return items;}uint32_t max_bytes()const{return bytes;}
 uint32_t page_bytes()const{return page;}uint64_t offset()const{return start;}bool release()const{return releasing;}};
struct Reply{bool accepted=false,complete=false,has_payload=false;uint32_t code=0,major=0,minor=0;uint64_t generation=0,offset=0,total=0;
 std::string nonce,df,dfhack,payload,snapshot,digest;
 void Clear(){*this=Reply();}void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){code=v;}
 void set_client_nonce(const std::string&v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}void set_protocol_minor(uint32_t v){minor=v;}
 void set_bridge_generation(uint64_t v){generation=v;}void set_df_version(const std::string&v){df=v;}void set_dfhack_version(const std::string&v){dfhack=v;}
 void set_observation(const std::string&v){payload=v;has_payload=true;}void set_snapshot_token(const std::string&v){snapshot=v;}
 void set_offset(uint64_t v){offset=v;}void set_total_bytes(uint64_t v){total=v;}void set_payload_sha256(const std::string&v){digest=v;}void set_complete(bool v){complete=v;}};
}
'''
DRIVER = r'''
#include <fstream>
#include <iostream>
#include <stdexcept>
#include "PRODUCER"
int checks=0;void check(bool value){++checks;if(!value)throw std::runtime_error("check "+std::to_string(checks));}
std::string hex(const std::string&v){std::string r;for(unsigned char b:v){r.push_back("0123456789abcdef"[b>>4]);r.push_back("0123456789abcdef"[b&15]);}return r;}
int main(int argc,char**argv){
 check(argc==2);setenv("DFMCP_OPERATIONS_PAGED_TOKEN",std::string(32,'t').c_str(),1);
 df::world world;int32_t nj=0,nb=0,ni=40000;df::global::world=&world;df::global::job_next_id=&nj;df::global::building_next_id=&nb;df::global::item_next_id=&ni;
 std::vector<df::item>items(40000);for(int i=0;i<40000;++i){items[i].id=i;items[i].flags.bits.on_ground=true;world.items.all.push_back(&items[i]);}
 color_ostream output;wire::Request request;wire::Reply reply;
 auto read=[&](){ReadObservation(output,&request,&reply);};
 auto rejected=[&](){read();check(!reply.accepted);check(reply.code!=0);check(!reply.has_payload);};
 Handshake(output,&request,&reply);check(reply.accepted);check(reply.minor==4);check(World::reads==0);
 read();check(reply.accepted);check(!reply.complete);check(reply.payload.size()==request.page);check(reply.total>2*1024*1024);
 const auto token=reply.snapshot,digest=reply.digest;const auto total=reply.total;const auto first=reply.payload;
 request.snapshot=token;read();check(reply.payload==first);check(reply.digest==digest);check(World::reads==1);
 // Underlying game can advance or its original pointers disappear: pages stay immutable.
 World::tick=100;items[0].stack=9;world.items.all.clear();std::string payload=first;std::size_t pages=1;
 while(payload.size()<total){request.start=payload.size();read();check(reply.accepted);check(reply.offset==payload.size());check(reply.digest==digest);
  check(reply.complete==(reply.offset+reply.payload.size()==total));payload+=reply.payload;++pages;check(pages<=1024);}
 check(World::reads==1);check(dfmcp_snapshot::sha256(payload)==digest);check(payload.substr(0,8)=="DFMO1400");
 read();check(reply.complete);check(World::reads==1); // Final page retries remain valid until release.
 request.nonce=std::string(16,'x');rejected();request.nonce=std::string(16,'n');
 request.items=1;rejected();request.items=65536;request.start=total;rejected();request.start=0;
 request.page=1;rejected();request.page=16384;request.minor=3;rejected();request.minor=4;
 request.token=std::string(32,'x');rejected();request.token=std::string(32,'t');
 request.releasing=true;read();check(reply.accepted);check(!reply.has_payload);check(snapshots.bytes()==0);
 request.releasing=false;rejected();check(World::reads==1);request.snapshot.clear();
 // Starting a new snapshot is explicit, never a continuation fallback.
 read();check(reply.accepted);check(reply.complete);check(reply.snapshot!=token);check(World::reads==2);
 request.snapshot=reply.snapshot;plugin_onstatechange(output,SC_WORLD_UNLOADED);rejected();check(snapshots.count()==0);
 request.snapshot.clear();request.start=1;rejected();request.start=0;
 world.items.all={&items[0]};items[0].container=&items[0];rejected();check(snapshots.count()==0);items[0].container=nullptr;
 world.items.all.push_back(nullptr);rejected();world.items.all.pop_back();
 request.items=32768;world.items.all.clear();for(auto&item:items)world.items.all.push_back(&item);rejected();request.items=65536;
 request.bytes=1024;rejected();check(snapshots.count()==0);request.bytes=16*1024*1024;
 // Exact maximum capture succeeds; an extra member is rejected without publishing.
 world.items.all.clear();items.resize(65536);ni=65536;
 for(int i=0;i<65536;++i){items[i].id=i;items[i].stack=1;items[i].flags.bits.on_ground=true;world.items.all.push_back(&items[i]);}
 read();check(reply.accepted);check(reply.total==3604550);check(!reply.complete);
 request.snapshot=reply.snapshot;request.releasing=true;read();check(reply.accepted);check(snapshots.count()==0);
 request.snapshot.clear();request.releasing=false;world.items.all.push_back(world.items.all.front());rejected();check(snapshots.count()==0);
 auto*service=plugin_rpcconnect(output);check(service->names==std::vector<std::string>({"Handshake","ReadObservation"}));check(service->flags==std::vector<int>({0,0}));delete service;
 std::ofstream file(argv[1],std::ios::binary);file.write(payload.data(),static_cast<std::streamsize>(payload.size()));file.close();check(bool(file));
 std::cout<<"{\"checks\":"<<checks<<",\"pages\":"<<pages<<",\"bytes\":"<<total<<",\"sha256\":\""<<hex(digest)<<"\"}\n";
}
'''

def expected_frame(count=40000):
    u=lambda n:struct.pack('>I',n)
    i=lambda n:struct.pack('>i',n)
    t=lambda v:struct.pack('>H',len(v))+v
    jobs=b'DFMJ1200'+u(105)+u(3)+b'\1'+i(1)+u(0)+t(b'region1')+u(0)
    out=b'DFMO1400'+u(len(jobs))+jobs+u(0)+u(count)+u(0)+u(count)
    row=i(1)+t(b'item_type_1')+i(-1)+i(0)+i(1)+u(1)+i(0)*3+u(64)+b'\0\0'
    return out+b''.join(u(n)+row for n in range(count))+u(0)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',default='g++');parser.add_argument('--output');args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1]
    producer=root/'bridge/dfhack-operations-v1_4/dfmcp_operations_v1_4.cpp'
    header=root/'bridge/common/retained_snapshot.h'
    with tempfile.TemporaryDirectory(prefix='dfmcp-paging-mock-') as work:
        work=pathlib.Path(work);(work/'mock.hpp').write_text(MOCK)
        headers=['Core.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','MiscUtils.h','DfmcpOperationsV1_4.pb.h',
                 'modules/Job.h','modules/Items.h','modules/World.h','df/building.h','df/building_type.h','df/global_objects.h',
                 'df/item.h','df/item_type.h','df/job.h','df/job_item_ref.h','df/job_list_link.h','df/unit.h','df/world.h']
        for name in headers:
            path=work/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('#include "mock.hpp"\n')
        (work/'test.cpp').write_text(DRIVER.replace('PRODUCER',str(producer)))
        flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic','-O2']
        subprocess.run([args.compiler,*flags,'-I',str(work),str(work/'test.cpp'),'-o',str(work/'test')],check=True,timeout=60)
        run=subprocess.run([str(work/'test'),str(work/'payload.bin')],check=True,capture_output=True,text=True,timeout=30)
        result=json.loads(run.stdout);expected=expected_frame();actual=(work/'payload.bin').read_bytes()
        assert actual==expected and hashlib.sha256(expected).hexdigest()==result['sha256']
        cache_test=root/'bridge/common/tests/retained_snapshot_test.cpp'
        subprocess.run([args.compiler,*flags,str(cache_test),'-o',str(work/'cache_test')],check=True,timeout=60)
        cache_run=subprocess.run([str(work/'cache_test')],capture_output=True,text=True,check=True,timeout=30)
        vectors=0
        for line in cache_run.stdout.splitlines():
            if ':' in line:
                length,digest=line.split(':');assert hashlib.sha256(b'x'*int(length)).hexdigest()==digest;vectors+=1
        assert vectors==11 and 'cache checks passed' in cache_run.stdout
        result.update(cache_lifecycle_checks='passed',independent_sha256_vectors=vectors,
            compiler=args.compiler,flags=flags,independent_python_bytes_equal=True,
            producer_sha256=hashlib.sha256(producer.read_bytes()).hexdigest(),cache_sha256=hashlib.sha256(header.read_bytes()).hexdigest(),
            evidence='actual_source_mock_interfaces_not_real_dfhack')
        if args.output:pathlib.Path(args.output).write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result,indent=2))
if __name__=='__main__':main()
