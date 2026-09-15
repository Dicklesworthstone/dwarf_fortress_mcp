#!/usr/bin/env python3
"""Compile the actual spatial producer against explicit mock native interfaces."""
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
inline std::string mock_key(const char *kind,int value){return std::string(kind)+"_"+std::to_string(value);}
namespace df {
struct coord{int32_t x=0,y=0,z=0;};
enum class building_type{Workshop=1,Stockpile=2};
enum class item_type{Drink=1,Barrel=2,Wood=3};
enum class job_type{BrewDrink=1};
enum class job_role_type{Reagent=1};
struct building{int32_t id=0,x1=0,y1=0,x2=0,y2=0,z=0,stage=1,maximum=3;building_type type=building_type::Workshop;
 building_type getType(){return type;}int32_t getBuildStage(){return stage;}int32_t getMaxBuildStage(){return maximum;}};
struct item{int32_t id=0,subtype=-1,material=0,material_index=1,stack=1;coord pos;item_type type=item_type::Drink;
 struct{struct{bool forbid=false,in_job=false,dump=false,removed=false,rotten=false,trader=false,on_ground=false,in_inventory=false,in_building=false;}bits;}flags;
 std::vector<void *> general_refs;item *container=nullptr;building *holder=nullptr;
 item_type getType(){return type;}int32_t getSubtype(){return subtype;}int32_t getMaterial(){return material;}
 int32_t getMaterialIndex(){return material_index;}int32_t getStackSize(){return stack;}};
struct unit{int32_t id=9;};
struct job_item_ref{df::item *item=nullptr;job_role_type role=job_role_type::Reagent;int32_t job_item_idx=0;};
struct job{int32_t id=7,completion_timer=-1;df::job_type job_type=df::job_type::BrewDrink;std::string reaction_name="BREW";coord pos;
 struct{struct{bool suspend=false,repeat=false;}bits;}flags;std::vector<job_item_ref *>items;
 struct{std::vector<void *>elements;}job_items;std::vector<void *>general_refs;unit *worker=nullptr;building *holder=nullptr;};
struct job_list_link{df::job *item=nullptr;job_list_link *next=nullptr;};
struct world{struct{job_list_link list;}jobs;struct{std::vector<building *>all;}buildings;struct{std::vector<item *>all;}items;};
namespace global{inline df::world *world=nullptr;inline int32_t *job_next_id=nullptr,*building_next_id=nullptr,*item_next_id=nullptr;}
enum class tiletype_shape{EMPTY=1,WALL=2,FLOOR=3,RAMP=4,RAMP_TOP=5,STAIR_UP=6,STAIR_DOWN=7,STAIR_UPDOWN=8};
enum class tiletype{Floor=3,Wall=2};
struct tile_designation{struct{bool hidden=false,liquid_type=false;unsigned flow_size=0,traffic=0,dig=0;}bits;};
struct tile_occupancy{struct{unsigned building=0;bool unit=false,unit_grounded=false;}bits;};
struct map_block{df::tiletype tiletype[16][16];tile_designation designation[16][16];tile_occupancy occupancy[16][16];
 uint32_t walkable[16][16];uint16_t temperature_1[16][16],temperature_2[16][16];
 map_block(){for(int x=0;x<16;++x)for(int y=0;y<16;++y){tiletype[x][y]=df::tiletype::Floor;walkable[x][y]=1;temperature_1[x][y]=temperature_2[x][y]=10015;}}};
}
namespace DFHack {
struct color_ostream{};struct PluginCommand{};enum command_result{CR_OK,CR_FAILURE};enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_OTHER};
struct VersionInfo{std::string getVersion(){return "df";}};
struct Core{std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();bool loaded=true;
 static Core &getInstance(){static Core c;return c;}bool isWorldLoaded(){return loaded;}};
namespace Version{inline std::string dfhack_version(){return "dfhack";}}
namespace World{inline uint32_t year=105,tick=3;inline int32_t site=1;inline bool fortress=true,paused=true;inline std::string folder="region1";
 inline uint32_t ReadCurrentYear(){return year;}inline uint32_t ReadCurrentTick(){return tick;}inline int32_t GetCurrentSiteId(){return site;}
 inline bool isFortressMode(){return fortress;}inline bool ReadPauseState(){return paused;}inline std::string ReadWorldFolder(){return folder;}}
namespace Job{inline df::unit *getWorker(df::job *j){return j->worker;}inline df::building *getHolder(df::job *j){return j->holder;}}
namespace Items{inline df::item *getContainer(df::item *i){return i->container;}inline df::building *getHolderBuilding(df::item *i){return i->holder;}}
namespace Maps{inline df::map_block block;inline int32_t sx=32,sy=32,sz=8;inline bool missing=false;inline unsigned reads=0;
 inline void getTileSize(int32_t &x,int32_t &y,int32_t &z){x=sx;y=sy;z=sz;}
 inline df::map_block *getTileBlock(int32_t,int32_t,int32_t){++reads;return missing?nullptr:&block;}}
inline bool is_valid_enum_item(df::tiletype t){return static_cast<int>(t)>=0&&static_cast<int>(t)<=8;}
inline df::tiletype_shape tileShape(df::tiletype t){return static_cast<df::tiletype_shape>(t);}
struct RPCService{std::vector<std::string>names;std::vector<int>flags;
 template<class F>void addFunction(const char *name,F,int flag){names.push_back(name);flags.push_back(flag);}};
}
namespace dfmcp::spatial::v1_6 {
struct Request{std::string token=std::string(32,'t'),nonce=std::string(16,'n'),snapshot;
 uint32_t major=1,minor=6,jobs=4096,buildings=4096,items=65536,bytes=16*1024*1024,page=16384;
 uint32_t ox=0,oy=0,oz=5,sx=4,sy=4,sz=1;uint64_t off=0;bool releasing=false;
 const std::string &bearer_token()const{return token;}const std::string &client_nonce()const{return nonce;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}
 uint32_t max_jobs()const{return jobs;}uint32_t max_buildings()const{return buildings;}uint32_t max_items()const{return items;}
 uint32_t max_bytes()const{return bytes;}uint32_t page_bytes()const{return page;}uint64_t offset()const{return off;}
 const std::string &snapshot_token()const{return snapshot;}bool release()const{return releasing;}
 uint32_t x()const{return ox;}uint32_t y()const{return oy;}uint32_t z()const{return oz;}
 uint32_t width()const{return sx;}uint32_t height()const{return sy;}uint32_t depth()const{return sz;}};
struct Reply{bool accepted=false,has_payload=false,complete=false;uint32_t code=0,major=0,minor=0;uint64_t generation=0,offset=0,total=0;
 std::string nonce,df,dfhack,payload,token,digest;
 void Clear(){*this=Reply();}void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){code=v;}
 void set_client_nonce(const std::string &v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}void set_protocol_minor(uint32_t v){minor=v;}
 void set_bridge_generation(uint64_t v){generation=v;}void set_df_version(const std::string &v){df=v;}void set_dfhack_version(const std::string &v){dfhack=v;}
 void set_observation(const std::string &v){payload=v;has_payload=true;}void set_snapshot_token(const std::string &v){token=v;}
 void set_page_offset(uint64_t v){offset=v;}void set_total_bytes(uint64_t v){total=v;}void set_payload_sha256(const std::string &v){digest=v;}
 void set_complete(bool v){complete=v;}};
}
'''
DRIVER=r'''
#include <iostream>
#include <stdexcept>
#include "PRODUCER"
unsigned checks=0;
void check(bool v){++checks;if(!v)throw std::runtime_error("check "+std::to_string(checks));}
void print_hex(const std::string &v){const char *h="0123456789abcdef";for(unsigned char c:v)std::cout<<h[c>>4]<<h[c&15];}
int main(){
 setenv("DFMCP_SPATIAL_TOKEN",std::string(32,'t').c_str(),1);
 df::world world;df::global::world=&world;int32_t nj=8,nb=22,ni=33;df::global::job_next_id=&nj;df::global::building_next_id=&nb;df::global::item_next_id=&ni;
 df::building workshop,stockpile;workshop.id=20;workshop.x1=1;workshop.y1=2;workshop.x2=3;workshop.y2=4;workshop.z=5;
 stockpile.id=21;stockpile.type=df::building_type::Stockpile;stockpile.stage=2;stockpile.maximum=2;
 df::item drink,barrel,wood;drink.id=30;drink.stack=5;drink.container=&barrel;drink.flags.bits.in_job=true;
 barrel.id=31;barrel.type=df::item_type::Barrel;barrel.holder=&workshop;barrel.flags.bits.in_building=true;
 wood.id=32;wood.type=df::item_type::Wood;wood.flags.bits.on_ground=true;wood.pos={1,2,5};
 df::job j;j.holder=&workshop;j.pos={1,2,5};df::job_item_ref a;a.item=&drink;j.items={&a};j.job_items.elements={&j};
 df::job_list_link link;link.item=&j;world.jobs.list.next=&link;world.buildings.all={&stockpile,&workshop};world.items.all={&wood,&barrel,&drink};
 Maps::block.designation[2][2].bits.hidden=true;
 wire::Request r;wire::Reply p;color_ostream out;
 auto read=[&](){ReadObservation(out,&r,&p);};
 auto bad=[&](){read();check(!p.accepted);check(p.code!=0);check(!p.has_payload);check(p.payload.empty());};
 Handshake(out,&r,&p);check(p.accepted);check(!p.has_payload);check(p.minor==6);
 read();check(p.accepted);check(p.complete);check(p.has_payload);check(p.total==p.payload.size());
 const auto golden=p.payload;const auto first_token=p.token;r.snapshot=p.token;
 const auto calls=Maps::reads;
 Maps::block.designation[2][2].bits.flow_size=7;Maps::block.temperature_1[2][2]=1;wood.stack=9;World::tick=4;
 read();check(p.accepted);check(p.payload==golden);check(Maps::reads==calls);
 r.nonce=std::string(16,'x');bad();r.nonce=std::string(16,'n');r.ox=1;bad();r.ox=0;r.sx=3;bad();r.sx=4;
 r.items=65535;bad();r.items=65536;r.off=golden.size();bad();r.off=0;
 r.releasing=true;read();check(p.accepted);check(!p.has_payload);check(p.token==first_token);read();check(!p.accepted);
 r.releasing=false;r.snapshot.clear();wood.stack=1;World::tick=3;
 read();check(p.accepted);check(p.token!=first_token);const auto hidden=p.payload;
 Maps::block.tiletype[2][2]=df::tiletype::Wall;Maps::block.walkable[2][2]=0;
 snapshots.clear();r.snapshot.clear();read();check(p.accepted);check(p.payload==hidden);
 snapshots.clear();r.token=std::string(32,'x');bad();r.token=std::string(32,'t');r.minor=5;bad();r.minor=6;
 r.nonce="short";bad();r.nonce=std::string(16,'n');r.sx=0;bad();r.sx=4;r.ox=32768;bad();r.ox=0;
 r.page=1;bad();r.page=16384;r.ox=30;bad();r.ox=0;
 r.items=2;bad();r.items=65536;j.items.push_back(&a);bad();j.items.pop_back();
 barrel.container=&drink;bad();barrel.container=nullptr;drink.stack=-1;bad();drink.stack=5;
 a.job_item_idx=1;bad();a.job_item_idx=0;link.next=&link;bad();link.next=nullptr;
 Maps::missing=true;read();check(p.accepted);check(p.payload.size()<golden.size());Maps::missing=false;snapshots.clear();
 read();check(p.accepted);r.snapshot=p.token;plugin_onstatechange(out,SC_WORLD_UNLOADED);bad();r.snapshot.clear();
 // A large immutable capture includes both inventory and terrain before paging.
 std::vector<df::item> many(40000);world.items.all.clear();world.buildings.all.clear();world.jobs.list.next=nullptr;ni=40000;
 for(int i=0;i<40000;++i){many[i].id=i;many[i].type=df::item_type::Wood;many[i].flags.bits.on_ground=true;many[i].pos={1,2,5};world.items.all.push_back(&many[i]);}
 std::string expected;check(capture::capture(bounds(&r),expected)==0);read();check(p.accepted);check(!p.complete);
 const auto large_digest=p.digest;const auto large_token=p.token;std::string assembled=p.payload;unsigned pages=1;
 many[0].stack=20;World::tick=9;Maps::block.designation[0][0].bits.flow_size=3;const auto captured_reads=Maps::reads;
 while(!p.complete){r.snapshot=large_token;r.off=assembled.size();read();check(p.accepted);check(p.token==large_token);check(p.digest==large_digest);check(p.offset==assembled.size());
  assembled+=p.payload;++pages;check(pages<1024);}
 check(assembled==expected);check(Maps::reads==captured_reads);check(dfmcp_snapshot::sha256(assembled)==large_digest);
 r.off=0;r.releasing=true;read();check(p.accepted);check(snapshots.count()==0);
 r.releasing=false;r.snapshot.clear();r.bytes=1024;bad();check(snapshots.count()==0);r.bytes=16*1024*1024;
 world.items.all.resize(65537,&wood);r.items=65536;bad();world.items.all.clear();
 auto *service=plugin_rpcconnect(out);check(service->names==std::vector<std::string>({"Handshake","ReadObservation"}));check(service->flags==std::vector<int>({0,0}));delete service;
 std::cout<<"{\"checks\":"<<checks<<",\"large_pages\":"<<pages<<",\"large_bytes\":"<<assembled.size()<<",\"large_sha256\":\"";
 print_hex(large_digest);std::cout<<"\",\"golden_hex\":\"";print_hex(golden);std::cout<<"\"}\n";
}
'''

def op_payload(large=False):
    u=lambda n:struct.pack('>I',n)
    i=lambda n:struct.pack('>i',n)
    t=lambda s:struct.pack('>H',len(s.encode()))+s.encode()
    ref=lambda n:b'\0' if n is None else b'\1'+u(n)
    jobs=b'DFMJ1200'+u(105)+u(3)+b'\1'+i(1)+u(8)+t('region1')+u(0 if large else 1)
    if not large:jobs+=u(7)+i(1)+t('job_type_1')+t('BREW')+b'\0\0'+i(1)+i(2)+i(5)+ref(None)+ref(20)+i(-1)+u(1)+u(1)
    out=b'DFMO1400'+u(len(jobs))+jobs+u(22)+u(40000 if large else 33)+u(0 if large else 2)
    if not large:
        for ident,typ,coords,stage,maximum in [(20,1,[1,2,3,4,5],1,3),(21,2,[0,0,0,0,0],2,2)]:
            out+=u(ident)+i(typ)+t(f'building_type_{typ}')+b''.join(i(v) for v in coords)+i(stage)+i(maximum)
    rows=((n,3,1,[1,2,5],64,None,None) for n in range(40000)) if large else [(30,1,5,[0,0,0],2,31,None),(31,2,1,[0,0,0],256,None,20),(32,3,1,[1,2,5],64,None,None)]
    parts=[out,u(40000 if large else 3)]
    for ident,typ,stack,pos,flags,container,holder in rows:
        parts.append(u(ident)+i(typ)+t(f'item_type_{typ}')+i(-1)+i(0)+i(1)+u(stack)+b''.join(i(v) for v in pos)+u(flags)+ref(container)+ref(holder))
    return b''.join(parts)+u(0 if large else 1)+(b'' if large else u(7)+u(30)+i(1)+i(0))

def golden(large=False):
    u=lambda n:struct.pack('>I',n)
    t=lambda s:struct.pack('>H',len(s))+s
    terrain=b'DFMM1500'+u(105)+u(3)+b'\1'+u(1)+t(b'region1')
    terrain+=b''.join(u(n) for n in [32,32,8,0,0,5,4,4,1,16])
    for y in range(4):
        for x in range(4):
            terrain+=b'\1' if (x,y)==(2,2) else b'\2'+u(3)+bytes([3,0,0,0,0,0,0])+u(1)+struct.pack('>HH',10015,10015)
    operations=op_payload(large)
    return b'DFMS1600'+u(len(operations))+operations+u(len(terrain))+terrain

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',default='g++');parser.add_argument('--output');args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1]
    producer=root/'bridge/dfhack-spatial-v1_6/dfmcp_spatial_v1_6.cpp'
    headers=['Core.h','MiscUtils.h','TileTypes.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','DfmcpSpatialV1_6.pb.h',
        'modules/Job.h','modules/Items.h','modules/Maps.h','modules/World.h','df/building.h','df/building_type.h','df/global_objects.h',
        'df/item.h','df/item_type.h','df/job.h','df/job_item_ref.h','df/job_list_link.h','df/unit.h','df/world.h','df/map_block.h',
        'df/tile_designation.h','df/tile_occupancy.h','df/tiletype_shape.h']
    with tempfile.TemporaryDirectory(prefix='dfmcp-spatial-mock-') as directory:
        directory=pathlib.Path(directory);(directory/'mock.hpp').write_text(MOCK)
        for name in headers:
            path=directory/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('#include "mock.hpp"\n')
        (directory/'test.cpp').write_text(DRIVER.replace('PRODUCER',str(producer)))
        flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic']
        subprocess.run([args.compiler,*flags,'-I',str(directory),str(directory/'test.cpp'),'-o',str(directory/'test')],check=True,timeout=60)
        result=json.loads(subprocess.run([str(directory/'test')],capture_output=True,text=True,check=True,timeout=30).stdout)
    expected=golden();assert result['golden_hex']==expected.hex()
    large=golden(True);assert result['large_sha256']==hashlib.sha256(large).hexdigest();assert result['large_bytes']==len(large)
    fixture=root/'crates/dfmcp-adapter/tests/fixtures/spatial_v1_6.hex'
    if fixture.exists():assert fixture.read_text().strip()==expected.hex()
    result.update(compiler=args.compiler,flags=flags,golden_bytes=len(expected),evidence='mock_native_interfaces_not_real_dfhack_qualification',
        source_sha256={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [producer,root/'bridge/common/spatial_capture.h',root/'bridge/common/retained_snapshot.h']})
    if args.output:pathlib.Path(args.output).write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))
if __name__=='__main__':main()
