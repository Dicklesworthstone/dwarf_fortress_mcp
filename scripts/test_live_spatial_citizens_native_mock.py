#!/usr/bin/env python3
"""Compile spatial/1.8 against explicit mocks; this is not native qualification."""
import argparse, hashlib, json, pathlib, runpy, subprocess, tempfile

DRIVER=r'''
#include <iostream>
#include <stdexcept>
#include "PRODUCER"
unsigned checks=0;void check(bool v){++checks;if(!v)throw std::runtime_error("check "+std::to_string(checks));}
uint32_t u32(const std::string&s,size_t p){return (uint32_t(uint8_t(s[p]))<<24)|(uint32_t(uint8_t(s[p+1]))<<16)|(uint32_t(uint8_t(s[p+2]))<<8)|uint8_t(s[p+3]);}
int main(){
 setenv("DFMCP_SPATIAL_CITIZEN_TOKEN",std::string(32,'t').c_str(),1);
 df::world world;df::global::world=&world;int32_t nj=8,nb=22,ni=33;df::global::job_next_id=&nj;df::global::building_next_id=&nb;df::global::item_next_id=&ni;
 df::building workshop;workshop.id=20;workshop.x1=1;workshop.y1=2;workshop.x2=3;workshop.y2=4;workshop.z=5;
 df::item wood;wood.id=32;wood.type=df::item_type::Wood;wood.flags.bits.on_ground=true;wood.pos={1,2,5};
 df::unit dwarf;dwarf.id=42;dwarf.pos={1,1,5};dwarf.name="Urist";dwarf.race="dwarf";
 df::job job;job.worker=&dwarf;job.holder=&workshop;job.pos={1,2,5};df::job_list_link link;link.item=&job;world.jobs.list.next=&link;
 world.buildings.all={&workshop};world.items.all={&wood};DFHack::Units::citizens={&dwarf};
 wire::Request r;wire::Reply p;color_ostream out;
 auto read=[&](){ReadObservation(out,&r,&p);};auto bad=[&](){read();check(!p.accepted);check(p.code!=0);check(!p.has_payload);};
 Handshake(out,&r,&p);check(p.accepted);check(p.minor==8);check(!p.has_payload);
 read();check(p.accepted);check(p.has_payload);check(p.complete);check(p.payload.compare(0,8,"DFMS1800")==0);
 const auto first=p.payload;const auto token=p.token;const auto base=u32(first,8);const auto citizen_len=u32(first,12+base);
 check(first.compare(16+base,8,"DFMC1800")==0);check(citizen_len>=20);check(u32(first,24+base)==1);check(u32(first,28+base)==42);
 check(first.find("job_skill_0",16+base)!=std::string::npos);check(first.find("job_skill_2",16+base)!=std::string::npos);
 // Repeat reads of an existing token return exact retained bytes after citizen/game changes.
 r.snapshot=token;dwarf.name="Domas";dwarf.pos={9,9,5};dwarf.mining=1;dwarf.stress=0;dwarf.available=false;World::tick=4;
 read();check(p.accepted);check(p.complete);check(p.payload==first);check(p.token==token);
 r.snapshot.clear();snapshots.clear();World::tick=3;read();check(p.accepted);check(p.payload!=first);check(p.payload.compare(0,8,"DFMS1800")==0);
 // Strict roster bounds and membership are fail closed.
 snapshots.clear();r.maxcitizens=0;bad();r.maxcitizens=4097;bad();r.maxcitizens=4096;
 df::unit other;other.id=43;DFHack::Units::citizens={&dwarf,&other};r.maxcitizens=1;bad();r.maxcitizens=4096;
 dwarf.resident=true;bad();dwarf.resident=false;DFHack::Units::citizens={&dwarf};
 dwarf.id=-1;bad();dwarf.id=42;dwarf.stress=7;bad();dwarf.stress=3;
 // Hidden terrain remains presence-only in the composite profile.
 Maps::block.designation[1][1].bits.hidden=true;snapshots.clear();read();check(p.accepted);const auto hidden=p.payload;
 Maps::block.tiletype[1][1]=df::tiletype::Wall;Maps::block.walkable[1][1]=0;snapshots.clear();read();check(p.accepted);check(p.payload==hidden);
 // World generation invalidates retained tokens and cache ownership binds citizen bounds.
 const auto retained=p.token;r.snapshot=retained;const auto old=generation;plugin_onstatechange(out,SC_WORLD_UNLOADED);check(generation==old+1);bad();r.snapshot.clear();
 r.minor=6;bad();r.minor=8;r.token=std::string(32,'x');bad();r.token=std::string(32,'t');
 auto*s=plugin_rpcconnect(out);check(s->names==std::vector<std::string>({"Handshake","ReadObservation"}));check(s->flags==std::vector<int>({0,0}));delete s;
 std::cout<<"{\"checks\":"<<checks<<",\"capture_bytes\":"<<first.size()<<",\"capture_sha256\":\"";
 const auto digest=dfmcp_snapshot::sha256(first);const char*h="0123456789abcdef";for(unsigned char c:digest)std::cout<<h[c>>4]<<h[c&15];std::cout<<"\"}\n";
}
'''

def transform_mock(mock:str)->str:
    mock=mock.replace('#define DFhackCExport\n', '#define DFhackCExport\n#define ENUM_LAST_ITEM(kind) df::kind::CARPENTRY\n')
    mock=mock.replace('enum class job_type{BrewDrink=1};', 'enum class job_type{BrewDrink=1};\nenum class job_skill{MINING=0,WOODCUTTING=1,CARPENTRY=2};')
    mock=mock.replace('struct unit{int32_t id=9;};', '''struct unit{int32_t id=9;coord pos;int32_t profession=3;std::string name="Urist",race="dwarf";
 int mining=7,woodcutting=0,carpentry=3,stress=3;bool available=true;bool alive=true,sane=true,active=true,visible=true,resident=false,baby=false,child=false,adult=true;};''')
    marker='namespace Items{inline df::item *getContainer(df::item *i){return i->container;}inline df::building *getHolderBuilding(df::item *i){return i->holder;}}'
    addition=marker+'''\nnamespace Units{inline std::vector<df::unit*> citizens;
 inline bool getCitizens(std::vector<df::unit*>&out,bool,bool){out=citizens;return true;}inline bool isCitizen(df::unit*,bool){return true;}
 inline bool isResident(df::unit*u,bool){return u->resident;}inline const std::string* getVisibleName(df::unit*u){return &u->name;}
 inline std::string getRaceReadableName(df::unit*u){return u->race;}inline int32_t getProfession(df::unit*u){return u->profession;}
 inline df::coord getPosition(df::unit*u){return u->pos;}inline bool isAlive(df::unit*u){return u->alive;}inline bool isSane(df::unit*u){return u->sane;}
 inline bool isActive(df::unit*u){return u->active;}inline bool isVisible(df::unit*u){return u->visible;}inline bool isBaby(df::unit*u){return u->baby;}
 inline bool isChild(df::unit*u){return u->child;}inline bool isAdult(df::unit*u){return u->adult;}
 inline int getStressCategory(df::unit*u){return u->stress;}inline bool isJobAvailable(df::unit*u,bool){return u->available;}
 inline int getNominalSkill(df::unit*u,df::job_skill s,bool){return s==df::job_skill::MINING?u->mining:s==df::job_skill::CARPENTRY?u->carpentry:u->woodcutting;}
 inline int getEffectiveSkill(df::unit*u,df::job_skill s){int n=getNominalSkill(u,s,true);return n?std::max(0,n-1):0;}
 inline int getExperience(df::unit*u,df::job_skill s,bool){return getNominalSkill(u,s,true)*100;}}
namespace Translation{inline std::string translateName(const std::string*n,bool){return n?*n:std::string();}}'''
    mock=mock.replace(marker,addition)
    mock=mock.replace('inline bool is_valid_enum_item(df::tiletype t){return static_cast<int>(t)>=0&&static_cast<int>(t)<=8;}',
        'inline bool is_valid_enum_item(df::tiletype t){return static_cast<int>(t)>=0&&static_cast<int>(t)<=8;}\ninline bool is_valid_enum_item(df::job_skill s){return static_cast<int>(s)>=0&&static_cast<int>(s)<=2;}')
    mock=mock.replace('namespace dfmcp::spatial::v1_6 {','namespace dfmcp::spatial::v1_8 {')
    mock=mock.replace('uint32_t major=1,minor=6,jobs=4096,buildings=4096,items=65536,bytes=16*1024*1024,page=16384;',
        'uint32_t major=1,minor=8,jobs=4096,buildings=4096,items=65536,bytes=16*1024*1024,page=16384,maxcitizens=4096;')
    mock=mock.replace('uint32_t width()const{return sx;}uint32_t height()const{return sy;}uint32_t depth()const{return sz;}};',
        'uint32_t width()const{return sx;}uint32_t height()const{return sy;}uint32_t depth()const{return sz;}uint32_t max_citizens()const{return maxcitizens;}};')
    return mock

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',default='g++');parser.add_argument('--output');args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1];base=runpy.run_path(str(root/'scripts/test_live_spatial_native_mock.py'));mock=transform_mock(base['MOCK'])
    producer=root/'bridge/dfhack-spatial-v1_8/dfmcp_spatial_v1_8.cpp'
    with tempfile.TemporaryDirectory(prefix='dfmcp-spatial18-mock-') as td:
        td=pathlib.Path(td);(td/'mock.hpp').write_text(mock)
        names=['Core.h','MiscUtils.h','TileTypes.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','modules/Job.h','modules/Items.h','modules/Maps.h','modules/World.h',
            'modules/Translation.h','modules/Units.h','df/building.h','df/building_type.h','df/global_objects.h','df/item.h','df/item_type.h','df/job.h','df/job_item_ref.h','df/job_list_link.h','df/job_skill.h','df/unit.h','df/world.h','df/map_block.h','df/tile_designation.h','df/tile_occupancy.h','df/tiletype_shape.h','df/coord.h','DfmcpSpatialV1_8.pb.h']
        for name in names:
            path=td/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('#include "mock.hpp"\n')
        (td/'test.cpp').write_text(DRIVER.replace('PRODUCER',str(producer)))
        flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic'];subprocess.run([args.compiler,*flags,'-I',str(td),str(td/'test.cpp'),'-o',str(td/'test')],check=True,timeout=60)
        result=json.loads(subprocess.run([str(td/'test')],capture_output=True,text=True,check=True,timeout=15).stdout)
        result.update(compiler=args.compiler,flags=flags,source_sha256=hashlib.sha256(producer.read_bytes()).hexdigest(),evidence='mock_interfaces_not_real_dfhack_or_rust_execution')
        if args.output:pathlib.Path(args.output).write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result,indent=2))
if __name__=='__main__':main()
