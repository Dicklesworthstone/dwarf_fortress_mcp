#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <string>
#include <string_view>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "TileTypes.h"
#include "modules/Maps.h"
#include "modules/World.h"
#include "df/map_block.h"
#include "df/tile_designation.h"
#include "df/tile_occupancy.h"
#include "df/tiletype_shape.h"
#include "DfmcpMapV1_5.pb.h"

using namespace DFHack;
namespace wire = dfmcp::map::v1_5;
DFHACK_PLUGIN("dfmcp_map_v1_5");
namespace {
constexpr std::uint64_t MAX_TILES=16384;
constexpr std::size_t MAX_PAYLOAD=1024*1024;
std::uint64_t generation=static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count())|1;
bool utf8(const std::string &v,std::size_t maximum) {
    if(v.empty()||v.size()>maximum)return false;
    for(std::size_t i=0;i<v.size();) {
        const auto c=static_cast<unsigned char>(v[i]);if(!c)return false;if(c<128){++i;continue;}
        const std::size_t n=c>=0xC2&&c<=0xDF?2:c>=0xE0&&c<=0xEF?3:c>=0xF0&&c<=0xF4?4:0;
        if(!n||i+n>v.size())return false;
        for(std::size_t j=1;j<n;++j)if((static_cast<unsigned char>(v[i+j])&0xC0)!=0x80)return false;
        const auto b=static_cast<unsigned char>(v[i+1]);
        if((c==0xE0&&b<0xA0)||(c==0xED&&b>=0xA0)||(c==0xF0&&b<0x90)||(c==0xF4&&b>=0x90))return false;
        i+=n;
    }return true;
}
void u32(std::string &out,std::uint32_t v){for(int shift=24;shift>=0;shift-=8)out.push_back(static_cast<char>((v>>shift)&255));}
void u16(std::string &out,std::uint16_t v){out.push_back(static_cast<char>(v>>8));out.push_back(static_cast<char>(v&255));}
void text(std::string &out,const std::string &v){u16(out,static_cast<std::uint16_t>(v.size()));out+=v;}
bool authorize(const wire::Request *in,wire::Reply *out) {
    out->Clear();out->set_accepted(false);out->set_failure_code(3);out->set_client_nonce("");
    out->set_protocol_major(1);out->set_protocol_minor(5);out->set_bridge_generation(0);
    out->set_df_version("");out->set_dfhack_version("");
    const std::uint32_t origin[]={in->x(),in->y(),in->z()},size[]={in->width(),in->height(),in->depth()};
    std::uint64_t volume=1;
    for(unsigned axis=0;axis<3;++axis){
        if(!size[axis]||size[axis]>128||origin[axis]>=32768||size[axis]>32768-origin[axis])return false;
        volume*=size[axis];
    }
    if(volume>MAX_TILES||in->max_bytes()<1024||in->max_bytes()>MAX_PAYLOAD
        ||in->client_nonce().size()<16||in->client_nonce().size()>64)return false;
    out->set_client_nonce(in->client_nonce());
    if(in->protocol_major()!=1||in->protocol_minor()!=5){out->set_failure_code(2);return false;}
    out->set_failure_code(1);
    const char *configured=std::getenv("DFMCP_MAP_TOKEN");
    const std::string_view expected=configured?std::string_view(configured):std::string_view();
    const auto &provided=in->bearer_token();
    if(expected.size()<32||expected.size()>256||provided.size()<32||provided.size()>256)return false;
    std::size_t diff=expected.size()^provided.size();
    for(std::size_t i=0;i<256;++i){const unsigned char a=i<expected.size()?expected[i]:0,b=i<provided.size()?provided[i]:0;diff|=a^b;}
    if(diff)return false;
    out->set_failure_code(5);
    const auto &version=Core::getInstance().vinfo;
    const std::string df=version?version->getVersion():std::string(),dfhack=Version::dfhack_version();
    if(!generation||generation==std::numeric_limits<std::uint64_t>::max()||!utf8(df,128)||!utf8(dfhack,128))return false;
    out->set_bridge_generation(generation);out->set_df_version(df);out->set_dfhack_version(dfhack);out->set_failure_code(0);return true;
}
std::uint8_t shape_tag(df::tiletype_shape shape){
    switch(shape){
        case df::tiletype_shape::EMPTY:return 1;case df::tiletype_shape::WALL:return 2;
        case df::tiletype_shape::FLOOR:return 3;case df::tiletype_shape::RAMP:return 4;
        case df::tiletype_shape::RAMP_TOP:return 5;case df::tiletype_shape::STAIR_UP:return 6;
        case df::tiletype_shape::STAIR_DOWN:return 7;case df::tiletype_shape::STAIR_UPDOWN:return 8;
        default:return 0;
    }
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(authorize(in,out))out->set_accepted(true);
    return CR_OK;
}
command_result ReadObservation(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(!authorize(in,out))return CR_OK;
    out->set_failure_code(4);
    if(!Core::getInstance().isWorldLoaded()||!World::isFortressMode())return CR_OK;
    std::int32_t sx=0,sy=0,sz=0;Maps::getTileSize(sx,sy,sz);
    const std::int32_t dimensions[]={sx,sy,sz};
    const std::uint32_t origin[]={in->x(),in->y(),in->z()},size[]={in->width(),in->height(),in->depth()};
    out->set_failure_code(5);
    for(unsigned axis=0;axis<3;++axis)if(dimensions[axis]<=0||dimensions[axis]>32768
        ||origin[axis]+size[axis]>static_cast<std::uint32_t>(dimensions[axis]))return CR_OK;
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear());const auto tick=World::ReadCurrentTick();
    const auto site=World::GetCurrentSiteId();const auto folder=World::ReadWorldFolder();
    if(year<0||year>std::numeric_limits<std::uint32_t>::max()||tick>=403200||site<0||!utf8(folder,512))return CR_OK;
    std::string payload("DFMM1500",8);u32(payload,static_cast<std::uint32_t>(year));u32(payload,tick);
    payload.push_back(World::ReadPauseState()?1:0);u32(payload,static_cast<std::uint32_t>(site));text(payload,folder);
    for(auto v:dimensions)u32(payload,static_cast<std::uint32_t>(v));
    for(auto v:origin)u32(payload,v);
    for(auto v:size)u32(payload,v);
    u32(payload,size[0]*size[1]*size[2]);
    // Flags=0 gives one suspended read. Never allocate a missing block or reveal
    // hidden terrain: those cells have presence tags only, not scrubbed attributes.
    for(std::uint32_t z=origin[2];z<origin[2]+size[2];++z)
    for(std::uint32_t y=origin[1];y<origin[1]+size[1];++y)
    for(std::uint32_t x=origin[0];x<origin[0]+size[0];++x){
        auto *block=Maps::getTileBlock(static_cast<std::int32_t>(x),static_cast<std::int32_t>(y),static_cast<std::int32_t>(z));
        if(!block){payload.push_back(0);continue;}
        const auto lx=x&15,ly=y&15;const auto &d=block->designation[lx][ly].bits;
        if(d.hidden){payload.push_back(1);continue;}
        const auto tt=block->tiletype[lx][ly];
        if(static_cast<int>(tt)<0||!is_valid_enum_item(tt))return CR_OK;
        const auto &o=block->occupancy[lx][ly].bits;
        payload.push_back(2);u32(payload,static_cast<std::uint32_t>(tt));
        payload.push_back(static_cast<char>(shape_tag(tileShape(tt))));
        payload.push_back(static_cast<char>(d.flow_size));payload.push_back(d.liquid_type?1:0);
        payload.push_back(static_cast<char>(d.traffic));payload.push_back(static_cast<char>(d.dig));
        payload.push_back(static_cast<char>(o.building));
        payload.push_back(static_cast<char>((o.unit?1:0)|(o.unit_grounded?2:0)));
        u32(payload,block->walkable[lx][ly]);u16(payload,block->temperature_1[lx][ly]);u16(payload,block->temperature_2[lx][ly]);
        if(payload.size()>in->max_bytes()){out->set_failure_code(3);return CR_OK;}
    }
    if(payload.size()>in->max_bytes()){out->set_failure_code(3);return CR_OK;}
    out->set_observation(payload);out->set_failure_code(0);out->set_accepted(true);return CR_OK;
}
}
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand> &){return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream &){return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event){
    if((event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED)&&generation!=std::numeric_limits<std::uint64_t>::max())++generation;
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &){auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("ReadObservation",ReadObservation,0);return s;}
