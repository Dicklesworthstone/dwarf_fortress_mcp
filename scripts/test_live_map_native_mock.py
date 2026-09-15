#!/usr/bin/env python3
"""Compile the actual map producer against mocks, never a real DFHack build."""
import argparse, hashlib, json, pathlib, struct, subprocess, tempfile

MOCK = r'''
#pragma once
#include <cstdint>
#include <memory>
#include <string>
#include <vector>
#define DFHACK_PLUGIN(name)
#define DFhackCExport
namespace df {
enum class tiletype_shape {OTHER=0,EMPTY=1,WALL=2,FLOOR=3,RAMP=4,RAMP_TOP=5,STAIR_UP=6,STAIR_DOWN=7,STAIR_UPDOWN=8};
enum class tiletype {OTHER=0,EMPTY=1,WALL=2,FLOOR=3,RAMP=4,RAMP_TOP=5,STAIR_UP=6,STAIR_DOWN=7,STAIR_UPDOWN=8};
struct tile_designation {struct {bool hidden=false,liquid_type=false;unsigned flow_size=0,traffic=0,dig=0;}bits;};
struct tile_occupancy {struct {unsigned building=0;bool unit=false,unit_grounded=false;}bits;};
struct map_block {df::tiletype tiletype[16][16];tile_designation designation[16][16];tile_occupancy occupancy[16][16];
 uint32_t walkable[16][16];uint16_t temperature_1[16][16],temperature_2[16][16];
 map_block(){for(int x=0;x<16;++x)for(int y=0;y<16;++y){tiletype[x][y]=df::tiletype::FLOOR;
 walkable[x][y]=1;temperature_1[x][y]=temperature_2[x][y]=10015;}}};
}
namespace DFHack {
struct color_ostream{};struct PluginCommand{};enum command_result{CR_OK,CR_FAILURE};
enum state_change_event{SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_OTHER};
struct VersionInfo {std::string getVersion(){return "df";}};
struct Core {std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();bool loaded=true;
 static Core& getInstance(){static Core c;return c;}bool isWorldLoaded(){return loaded;}};
namespace Version{inline std::string dfhack_version(){return "dfhack";}}
namespace World{inline uint32_t year=105,tick=3;inline int32_t site=1;inline bool paused=true,fortress=true;
 inline std::string folder="region1";inline uint32_t ReadCurrentYear(){return year;}inline uint32_t ReadCurrentTick(){return tick;}
 inline int32_t GetCurrentSiteId(){return site;}inline std::string ReadWorldFolder(){return folder;}
 inline bool ReadPauseState(){return paused;}inline bool isFortressMode(){return fortress;}}
namespace Maps{inline df::map_block blocks[8];inline int missing=7,calls=0;inline int32_t sx=32,sy=32,sz=3;
 inline void getTileSize(int32_t&x,int32_t&y,int32_t&z){x=sx;y=sy;z=sz;}
 inline df::map_block* getTileBlock(int32_t x,int32_t y,int32_t z){++calls;
 if(x<0||x>=32||y<0||y>=32||z<1||z>2)return nullptr;
 int id=(z-1)*4+(y/16)*2+x/16;return id==missing?nullptr:&blocks[id];}}
inline bool is_valid_enum_item(df::tiletype t){return static_cast<int>(t)>=0&&static_cast<int>(t)<=8;}
inline df::tiletype_shape tileShape(df::tiletype t){return static_cast<df::tiletype_shape>(t);}
struct RPCService{std::vector<std::string>names;std::vector<int>flags;
 template<class F>void addFunction(const char*n,F,int f){names.push_back(n);flags.push_back(f);}};
}
namespace dfmcp::map::v1_5 {
struct Request{std::string token=std::string(32,'t'),nonce=std::string(16,'n');
 uint32_t major=1,minor=5,ox=14,oy=15,oz=1,w=4,h=3,d=2,maximum=1024*1024;
 const std::string& bearer_token()const{return token;}const std::string&client_nonce()const{return nonce;}
 uint32_t protocol_major()const{return major;}uint32_t protocol_minor()const{return minor;}
 uint32_t x()const{return ox;}uint32_t y()const{return oy;}uint32_t z()const{return oz;}
 uint32_t width()const{return w;}uint32_t height()const{return h;}uint32_t depth()const{return d;}
 uint32_t max_bytes()const{return maximum;}};
struct Reply{bool accepted=false,has_payload=false;uint32_t code=0,major=0,minor=0;uint64_t generation=0;
 std::string nonce,df,dfhack,payload;
 void Clear(){*this=Reply();}void set_accepted(bool v){accepted=v;}void set_failure_code(uint32_t v){code=v;}
 void set_client_nonce(const std::string&v){nonce=v;}void set_protocol_major(uint32_t v){major=v;}
 void set_protocol_minor(uint32_t v){minor=v;}void set_bridge_generation(uint64_t v){generation=v;}
 void set_df_version(const std::string&v){df=v;}void set_dfhack_version(const std::string&v){dfhack=v;}
 void set_observation(const std::string&v){payload=v;has_payload=true;}};
}
'''
DRIVER = r'''
#include <iostream>
#include <stdexcept>
#include "PRODUCER"
int checks=0;void check(bool c){++checks;if(!c)throw std::runtime_error("check "+std::to_string(checks));}
int main(){setenv("DFMCP_MAP_TOKEN",std::string(32,'t').c_str(),1);
 wire::Request request;wire::Reply reply;color_ostream output;
 auto read=[&](){ReadObservation(output,&request,&reply);};
 auto reject=[&](){read();check(!reply.accepted);check(reply.code!=0);check(!reply.has_payload);check(reply.payload.empty());};
 Maps::blocks[0].designation[15][15].bits.hidden=true;
 Maps::blocks[0].tiletype[15][15]=static_cast<df::tiletype>(-1);
 Maps::blocks[0].designation[15][15].bits.flow_size=7;
 Maps::blocks[0].tiletype[14][15]=df::tiletype::STAIR_UP;
 Maps::blocks[4].tiletype[14][15]=df::tiletype::STAIR_DOWN;
 Maps::blocks[1].designation[0][15].bits.flow_size=3;Maps::blocks[1].designation[0][15].bits.liquid_type=true;
 Maps::blocks[2].occupancy[14][0].bits.building=2;Maps::blocks[2].occupancy[14][0].bits.unit_grounded=true;
 Handshake(output,&request,&reply);check(reply.accepted);check(!reply.has_payload);check(reply.minor==5);check(Maps::calls==0);
 read();check(reply.accepted);check(reply.has_payload);check(reply.code==0);check(Maps::calls==24);const auto golden=reply.payload;
 // Arbitrary changes behind the hidden cell never change the wire observation.
 Maps::blocks[0].tiletype[15][15]=df::tiletype::WALL;Maps::blocks[0].temperature_1[15][15]=60000;
 Maps::blocks[0].walkable[15][15]=999;read();check(reply.payload==golden);
 request.token="short";reject();request.token=std::string(32,'x');reject();request.token=std::string(32,'t');
 request.nonce="short";reject();request.nonce=std::string(16,'n');request.minor=4;reject();request.minor=5;
 request.ox=UINT32_MAX;reject();request.ox=14;request.w=0;reject();request.w=129;reject();request.w=4;
 request.w=request.h=request.d=128;reject();request.w=4;request.h=3;request.d=2;
 request.ox=31;reject();request.ox=14;request.maximum=1023;reject();request.maximum=1024*1024+1;reject();request.maximum=1024*1024;
 World::tick=403200;reject();World::tick=3;World::site=-1;reject();World::site=1;
 World::folder=std::string("a\0b",3);reject();World::folder=std::string("\xc0\x80",2);reject();World::folder="region1";
 Core::getInstance().loaded=false;reject();Core::getInstance().loaded=true;World::fortress=false;reject();World::fortress=true;
 Maps::sx=0;reject();Maps::sx=32;
 Maps::blocks[0].tiletype[14][15]=static_cast<df::tiletype>(99);reject();Maps::blocks[0].tiletype[14][15]=df::tiletype::STAIR_UP;
 Maps::blocks[0].designation[15][15].bits.hidden=false;read();check(reply.accepted);check(reply.payload!=golden);
 Maps::blocks[0].designation[15][15].bits.hidden=true;read();check(reply.payload==golden);
 // Unknown native shapes cannot silently become a floor.
 for(int n=0;n<=8;++n)check(shape_tag(static_cast<df::tiletype_shape>(n))==n);
 check(shape_tag(static_cast<df::tiletype_shape>(99))==0);
 const auto prior=generation;plugin_onstatechange(output,SC_WORLD_UNLOADED);read();check(reply.generation==prior+1);check(reply.payload==golden);
 auto*s=plugin_rpcconnect(output);check(s->names==std::vector<std::string>({"Handshake","ReadObservation"}));
 check(s->flags==std::vector<int>({0,0}));delete s;
 request.ox=0;request.oy=0;request.oz=1;request.w=32;request.h=32;request.d=2;request.maximum=1024;reject();
 request.maximum=1024*1024;read();check(reply.accepted);check(reply.payload.size()>1024);
 // Full hard-volume acquisition; sparse blocks remain unknown instead of being created.
 Maps::sx=128;Maps::sy=128;request.w=128;request.h=128;request.d=1;read();check(reply.accepted);check(reply.payload.size()>16384);
 const char*hex="0123456789abcdef";std::cout<<"{\"checks\":"<<checks<<",\"hex\":\"";
 for(unsigned char c:golden){std::cout<<hex[c>>4]<<hex[c&15];}std::cout<<"\"}\n";
}
'''

def golden_frame():
    u=lambda v:struct.pack('>I',v)
    h=lambda v:struct.pack('>H',v)
    out=b'DFMM1500'+u(105)+u(3)+b'\1'+u(1)+h(7)+b'region1'
    out+=b''.join(u(v) for v in [32,32,3,14,15,1,4,3,2])+u(24)
    for z in [1,2]:
        for y in range(15,18):
            for x in range(14,18):
                if z==2 and x>=16 and y>=16:out+=b'\0';continue
                if (x,y,z)==(15,15,1):out+=b'\1';continue
                shape=6 if (x,y,z)==(14,15,1) else 7 if (x,y,z)==(14,15,2) else 3
                liquid=3 if (x,y,z)==(16,15,1) else 0
                occupancy=2 if (x,y,z)==(14,16,1) else 0
                out+=b'\2'+u(shape)+bytes([shape,liquid,int(liquid>0),0,0,occupancy,occupancy])+u(1)+h(10015)+h(10015)
    return out

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--compiler',default='g++');parser.add_argument('--output');args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1];producer=root/'bridge/dfhack-map-v1_5/dfmcp_map_v1_5.cpp'
    with tempfile.TemporaryDirectory(prefix='dfmcp-map-mock-') as tmp:
        tmp=pathlib.Path(tmp);(tmp/'mock.hpp').write_text(MOCK)
        for name in ['Core.h','Export.h','PluginManager.h','RemoteServer.h','VersionInfo.h','TileTypes.h',
            'modules/Maps.h','modules/World.h','df/map_block.h','df/tile_designation.h','df/tile_occupancy.h',
            'df/tiletype_shape.h','DfmcpMapV1_5.pb.h']:
            path=tmp/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('#include "mock.hpp"\n')
        (tmp/'test.cpp').write_text(DRIVER.replace('PRODUCER',str(producer)))
        flags=['-std=c++17','-Wall','-Wextra','-Werror','-pedantic']
        subprocess.run([args.compiler,*flags,'-I',str(tmp),str(tmp/'test.cpp'),'-o',str(tmp/'test')],check=True,timeout=60)
        result=json.loads(subprocess.run([str(tmp/'test')],capture_output=True,text=True,check=True,timeout=15).stdout)
        expected=golden_frame();assert result['hex']==expected.hex()
        fixture=root/'crates/dfmcp-adapter/tests/fixtures/map_v1_5.hex'
        if fixture.exists():assert fixture.read_text().strip()==expected.hex()
        result.update(compiler=args.compiler,flags=flags,golden_bytes=len(expected),source_sha256=hashlib.sha256(producer.read_bytes()).hexdigest(),
            evidence='mock_interfaces_not_real_dfhack_or_rust_execution')
        if args.output:pathlib.Path(args.output).write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result,indent=2))
if __name__=='__main__':main()
