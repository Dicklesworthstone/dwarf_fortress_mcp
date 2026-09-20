#pragma once
// Explicit API doubles for compiling the actual plugin translation unit.
// These layouts are NOT DFHack ABI or protobuf serialization implementations.
#include <array>
#include <cstdint>
#include <map>
#include <memory>
#include <set>
#include <stdexcept>
#include <string>
#include <tuple>
#include <vector>
namespace mock {
inline bool loaded=true,mapped=true,fortress=true,paused=true;
inline bool unknown=false,initialized=true,reply_failure=false,reserve_failure=false,partial_failure=false;
inline unsigned events_alive=0,event_constructor_calls=0,fail_constructor_at=0;
inline std::uint32_t tick=12345;
inline std::int32_t year=0,site=1;
inline std::string folder="region1";
inline bool missing=false;
inline std::tuple<int,int,int> missing_block{};
}
namespace df {
struct coord {int x=0,y=0,z=0;};
enum class tile_dig_designation {No=0,Default=1,Channel=2};
enum class tiletype {StoneWall=1,SoilWall=2,MineralWall=3,ConstructedWall=4,Floor=5,Bad=99};
enum class tiletype_shape {WALL,FLOOR};
enum class tiletype_material {STONE,SOIL,MINERAL,CONSTRUCTION};
union tile_designation {
    std::uint32_t whole;
    struct {tile_dig_designation dig:3;std::uint32_t hidden:1,smooth:2,flow_size:3,feature_local:1,feature_global:1,water_table:1,unused:20;}bits;
    tile_designation():whole(0){}
};
union tile_occupancy {std::uint32_t whole;struct{std::uint32_t building:3,unit:1,unit_grounded:1,item:1,unused:26;}bits;tile_occupancy():whole(0){}};
union block_flags {std::uint32_t whole;struct{std::uint32_t designated:1,other:31;}bits;block_flags():whole(0){}};
struct block_square_event {virtual ~block_square_event()=default;};
struct block_square_event_designation_priorityst: block_square_event {
    std::uint32_t priority[16][16]{};
    block_square_event_designation_priorityst(){if(++mock::event_constructor_calls==mock::fail_constructor_at)throw std::bad_alloc();++mock::events_alive;}
    ~block_square_event_designation_priorityst() override{--mock::events_alive;}
};
struct EventVector:std::vector<block_square_event*>{
    void reserve(std::size_t n){if(mock::reserve_failure)throw std::bad_alloc();std::vector<block_square_event*>::reserve(n);}
};
struct map_block {
    std::array<std::array<tile_designation,16>,16> designation{};
    std::array<std::array<tile_occupancy,16>,16> occupancy{};
    std::array<std::array<df::tiletype,16>,16> tiletype{};
    std::array<std::array<std::uint16_t,16>,16> temperature_1{},temperature_2{};
    block_flags flags;std::int32_t dsgn_check_cooldown=25;EventVector block_events;
    map_block(){for(int x=0;x<16;++x)for(int y=0;y<16;++y){tiletype[x][y]=df::tiletype::StoneWall;temperature_1[x][y]=10015;temperature_2[x][y]=10015;}}
    ~map_block(){for(auto*p:block_events)delete p;}
};
struct job{std::int32_t id=0;coord pos;};
struct job_list_link{job*item=nullptr;job_list_link*next=nullptr;};
struct world{struct {job_list_link list;}jobs;};
namespace global {inline df::world *world=nullptr;}
}
namespace mock {
inline std::map<std::tuple<int,int,int>,std::unique_ptr<df::map_block>> blocks;
inline std::size_t designated_tiles(){std::size_t n=0;for(const auto&p:blocks)for(const auto&r:p.second->designation)for(const auto&d:r)n+=d.bits.dig==df::tile_dig_designation::Default;return n;}
}
namespace DFHack {
struct color_ostream{};struct PluginCommand{};
enum command_result{CR_OK,CR_FAILURE};
enum state_change_event{SC_MAP_LOADED,SC_MAP_UNLOADED,SC_WORLD_LOADED,SC_WORLD_UNLOADED,SC_PAUSED,SC_UNPAUSED,SC_OTHER};
struct VersionInfo{std::string getVersion()const{return "test-df";}};
struct Core{
    std::shared_ptr<VersionInfo>vinfo=std::make_shared<VersionInfo>();
    static Core&getInstance(){static Core c;return c;}
    bool isWorldLoaded()const{return mock::loaded;}bool isMapLoaded()const{return mock::mapped;}
};
namespace Version {inline const char*dfhack_version(){return "test-dfhack";}}
namespace World {
inline bool isFortressMode(){return mock::fortress;}inline bool ReadPauseState(){return mock::paused;}
inline int ReadCurrentYear(){return mock::year;}inline std::uint32_t ReadCurrentTick(){return mock::tick;}
inline int GetCurrentSiteId(){return mock::site;}inline std::string ReadWorldFolder(){return mock::folder;}
}
inline bool is_valid_enum_item(df::tiletype t){return t!=df::tiletype::Bad;}
inline df::tiletype_shape tileShape(df::tiletype t){return t==df::tiletype::Floor?df::tiletype_shape::FLOOR:df::tiletype_shape::WALL;}
inline df::tiletype_material tileMaterial(df::tiletype t){switch(t){case df::tiletype::SoilWall:return df::tiletype_material::SOIL;case df::tiletype::MineralWall:return df::tiletype_material::MINERAL;case df::tiletype::ConstructedWall:return df::tiletype_material::CONSTRUCTION;default:return df::tiletype_material::STONE;}}
namespace Maps {
inline bool IsValid(){return mock::mapped;}
inline void getTileSize(int&x,int&y,int&z){x=64;y=64;z=8;}
inline df::map_block *getTileBlock(int x,int y,int z){
    if(mock::partial_failure&&mock::designated_tiles()){mock::partial_failure=false;throw std::runtime_error("partial write fault");}
    auto key=std::make_tuple(x/16,y/16,z);
    if(mock::missing&&key==mock::missing_block)return nullptr;
    auto it=mock::blocks.find(key);return it==mock::blocks.end()?nullptr:it->second.get();
}
inline bool isTileAquifer(int x,int y,int z){auto*b=getTileBlock(x,y,z);return b&&b->designation[x&15][y&15].bits.water_table;}
inline bool SortBlockEvents(df::map_block*b,void*,void*,void*,void*,void*,void*,void*,std::vector<df::block_square_event_designation_priorityst*>*out){
    for(auto*e:b->block_events)if(auto*p=dynamic_cast<df::block_square_event_designation_priorityst*>(e))out->push_back(p);
    return true;
}
}
struct RPCService{std::vector<std::pair<std::string,int>>methods;template<class F>void addFunction(const char*n,F,int flags){methods.emplace_back(n,flags);}};
}
#define DFHACK_PLUGIN(name)
#define DFhackCExport extern "C"
namespace dfmcp::dig::v1_16 {
struct Request {
    unsigned mask=0;std::string bearer=std::string(32,'s'),nonce=std::string(16,'n');unsigned major=1,minor=16;
    std::uint32_t xx=15,yy=15,zz=2,ww=2,hh=2;bool hidden=false;
    std::string key="dig-001",witness,plan,token;
    struct Fields{bool empty()const{return !mock::unknown;}};
    struct Reflection{Fields GetUnknownFields(const Request&)const{return {};}};
    const Reflection*GetReflection()const{static Reflection r;return &r;}bool IsInitialized()const{return mock::initialized;}
    const std::string&bearer_token()const{return bearer;}const std::string&client_nonce()const{return nonce;}
    unsigned protocol_major()const{return major;}unsigned protocol_minor()const{return minor;}
#define FIELD(type,name,member,bit) bool has_##name()const{return (mask&(bit))!=0;} type name()const{return member;}
    FIELD(std::uint32_t,x,xx,1) FIELD(std::uint32_t,y,yy,2) FIELD(std::uint32_t,z,zz,4)
    FIELD(std::uint32_t,width,ww,8) FIELD(std::uint32_t,height,hh,16) FIELD(bool,allow_hidden_neighbors,hidden,32)
    FIELD(const std::string&,idempotency_key,key,64) FIELD(const std::string&,expected_witness,witness,128)
    FIELD(const std::string&,plan_digest,plan,256) FIELD(const std::string&,prepare_token,token,512)
#undef FIELD
};
struct Reply {
    bool accepted=false,replayed=false;std::uint32_t code=0,major=0,minor=0;std::uint64_t generation=0;
    std::string nonce,df_version,dfhack_version,observation,effect;unsigned mask=0;
    void Clear(){*this=Reply{};}void set_accepted(bool b){accepted=b;}void set_failure_code(unsigned n){code=n;}
    void set_client_nonce(const std::string&s){nonce=s;}void set_protocol_major(unsigned n){major=n;}void set_protocol_minor(unsigned n){minor=n;}
    void set_bridge_generation(std::uint64_t n){generation=n;}void set_df_version(const std::string&s){df_version=s;}
    void set_dfhack_version(const std::string&s){dfhack_version=s;}void set_observation(const std::string&s){mask|=1;observation=s;}
    void set_replayed(bool b){mask|=4;replayed=b;}
    void set_effect_record(const std::string&s){if(mock::reply_failure){mock::reply_failure=false;throw std::bad_alloc();}mask|=2;effect=s;}
};
}
