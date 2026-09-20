#pragma once
// Explicit semantic API doubles, NOT generated DFHack/protobuf ABI definitions.
#include <array>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>
namespace df {
enum tile_dig_designation : unsigned { No=0, Default=1, Channel=2 };
enum class tiletype : int { StoneWall=42, MineralWall, SoilWall, StoneFloor, SmoothWall, LavaWall, Construction };
enum class tiletype_shape { WALL, FLOOR, EMPTY };
enum class tiletype_material { STONE, MINERAL, SOIL, LAVA_STONE, CONSTRUCTION };
enum class tiletype_special { NORMAL, SMOOTH };
union tile_designation {
    std::uint32_t whole=0;
    struct { unsigned dig:3, hidden:1, water_table:1, feature_local:1, feature_global:1,
        smooth:2, flow_size:3, liquid_type:1, traffic:2, other:17; } bits;
};
union tile_occupancy {
    std::uint32_t whole=0;
    struct { unsigned building:3, unit:1, unit_grounded:1, item:1, dig_auto:1, heavy_aquifer:1, other:24; } bits;
};
union block_flags {
    std::uint32_t whole=0;
    struct { unsigned designated:1, has_aquifer:1, other:30; } bits;
};
struct map_block {
    std::array<std::array<df::tiletype,16>,16> tiletype{};
    std::array<std::array<tile_designation,16>,16> designation{};
    std::array<std::array<tile_occupancy,16>,16> occupancy{};
    std::array<std::array<std::uint16_t,16>,16> temperature_1{}, temperature_2{};
    block_flags flags;
};
}
namespace DFHack {
struct color_ostream {};
struct PluginCommand {};
enum command_result { CR_OK, CR_FAILURE };
enum state_change_event { SC_MAP_LOADED, SC_MAP_UNLOADED, SC_WORLD_LOADED, SC_WORLD_UNLOADED, SC_PAUSED, SC_UNPAUSED, SC_OTHER };
struct NativeVersion { std::string value="53.test"; std::string getVersion() const {return value;} };
struct Core {
    std::shared_ptr<NativeVersion> vinfo=std::make_shared<NativeVersion>();
    bool loaded=true, mapped=true;
    static Core &getInstance() {static Core c;return c;}
    bool isWorldLoaded() const {return loaded;}
    bool isMapLoaded() const {return mapped;}
};
namespace Version { inline std::string value="53.test-r1"; inline const char *dfhack_version(){return value.c_str();} }
namespace World {
    inline bool fort=true, paused=true;
    inline std::int64_t year=0, site=1;
    inline std::uint32_t tick=12345;
    inline std::string folder="region1";
    inline bool isFortressMode(){return fort;}
    inline bool ReadPauseState(){return paused;}
    inline std::int64_t ReadCurrentYear(){return year;}
    inline std::uint32_t ReadCurrentTick(){return tick;}
    inline std::int64_t GetCurrentSiteId(){return site;}
    inline std::string ReadWorldFolder(){return folder;}
}
namespace Maps {
    inline std::array<df::map_block,128> blocks{};
    inline std::array<bool,128> present{};
    inline unsigned calls=0, fail_at=0;
    inline std::int32_t sx=64,sy=64,sz=8;
    inline std::function<void(unsigned)> on_read;
    inline void getTileSize(std::int32_t &x,std::int32_t &y,std::int32_t &z){x=sx;y=sy;z=sz;}
    inline df::map_block *getTileBlock(std::int32_t x,std::int32_t y,std::int32_t z) {
        ++calls; if(on_read)on_read(calls);
        if(calls==fail_at || x<0 || x>=64 || y<0 || y>=64 || z<0 || z>=8)return nullptr;
        const auto n=static_cast<std::size_t>((z*4+y/16)*4+x/16);return present[n]?&blocks[n]:nullptr;
    }
}
inline bool is_valid_enum_item(df::tiletype t){return static_cast<int>(t)>=42 && static_cast<int>(t)<=48;}
inline df::tiletype_shape tileShape(df::tiletype t){return t==df::tiletype::StoneFloor?df::tiletype_shape::FLOOR:df::tiletype_shape::WALL;}
inline df::tiletype_material tileMaterial(df::tiletype t){
    if(t==df::tiletype::MineralWall)return df::tiletype_material::MINERAL;
    if(t==df::tiletype::SoilWall)return df::tiletype_material::SOIL;
    if(t==df::tiletype::LavaWall)return df::tiletype_material::LAVA_STONE;
    if(t==df::tiletype::Construction)return df::tiletype_material::CONSTRUCTION;
    return df::tiletype_material::STONE;
}
inline df::tiletype_special tileSpecial(df::tiletype t){return t==df::tiletype::SmoothWall?df::tiletype_special::SMOOTH:df::tiletype_special::NORMAL;}
struct RPCService {
    std::vector<std::pair<std::string,unsigned>> methods;
    template<class F> void addFunction(const std::string &name,F,unsigned flags){methods.emplace_back(name,flags);}
};
}
#define DFHACK_PLUGIN(name) [[maybe_unused]] static constexpr const char *plugin_name=name
#define DFhackCExport
namespace dfmcp::dig::v1_15 {
struct UnknownFields {bool unknown=false;bool empty() const{return !unknown;}};
struct Request;
struct Reflection {UnknownFields GetUnknownFields(const Request &) const;};
struct Request {
    unsigned required=0;
    bool unknown=false;
    std::string bearer,nonce;
    std::uint32_t major=0,minor=0;
    bool IsInitialized()const{return required==15;}
    const Reflection *GetReflection()const{static Reflection r;return &r;}
    const std::string &bearer_token()const{return bearer;} void set_bearer_token(const std::string &v){bearer=v;required|=1;}
    const std::string &client_nonce()const{return nonce;} void set_client_nonce(const std::string &v){nonce=v;required|=2;}
    std::uint32_t protocol_major()const{return major;} void set_protocol_major(std::uint32_t n){major=n;required|=4;}
    std::uint32_t protocol_minor()const{return minor;} void set_protocol_minor(std::uint32_t n){minor=n;required|=8;}
#define NUMBER(name) std::optional<std::uint32_t> name##_; bool has_##name()const{return name##_.has_value();} std::uint32_t name()const{return name##_.value_or(0);} void set_##name(std::uint32_t n){name##_=n;}
    NUMBER(x) NUMBER(y) NUMBER(z) NUMBER(width) NUMBER(height)
#undef NUMBER
#define TEXT(name) std::optional<std::string> name##_; bool has_##name()const{return name##_.has_value();} const std::string &name()const{static const std::string empty;return name##_?*name##_:empty;} void set_##name(const std::string &v){name##_=v;}
    TEXT(idempotency_key) TEXT(expected_witness) TEXT(plan_digest) TEXT(prepare_token)
#undef TEXT
};
inline UnknownFields Reflection::GetUnknownFields(const Request &r)const{return {r.unknown};}
struct Reply {
    bool accepted_=false;
    std::uint32_t code_=0,major_=0,minor_=0;
    std::uint64_t generation_=0;
    std::string nonce_,df_,dfhack_;
    std::optional<std::string> observation_,effect_;
    std::optional<bool> replayed_;
    static inline bool fail_effect=false;
    void Clear(){*this=Reply{};}
    void set_accepted(bool b){accepted_=b;} bool accepted()const{return accepted_;}
    void set_failure_code(std::uint32_t c){code_=c;} std::uint32_t failure_code()const{return code_;}
    void set_client_nonce(const std::string &v){nonce_=v;}
    void set_protocol_major(std::uint32_t n){major_=n;} void set_protocol_minor(std::uint32_t n){minor_=n;}
    void set_bridge_generation(std::uint64_t n){generation_=n;}
    void set_df_version(const std::string &v){df_=v;} void set_dfhack_version(const std::string &v){dfhack_=v;}
    void set_observation(const std::string &v){observation_=v;}
    void set_effect_record(const std::string &v){if(fail_effect){fail_effect=false;throw std::bad_alloc();}effect_=v;}
    void set_replayed(bool b){replayed_=b;}
};
}
