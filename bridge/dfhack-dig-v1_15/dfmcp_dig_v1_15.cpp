#include "../common/dig_designation.h"
#include <array>
#include <cstdlib>
#include <string_view>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "TileTypes.h"
#include "modules/Maps.h"
#include "modules/World.h"
#include "df/map_block.h"
#include "df/tile_dig_designation.h"
#include "df/tiletype_material.h"
#include "df/tiletype_shape.h"
#include "df/tiletype_special.h"
#include "DfmcpDigV1_15.pb.h"

using namespace DFHack;
namespace wire = dfmcp::dig::v1_15;
namespace dg = dfmcp_dig;
DFHACK_PLUGIN("dfmcp_dig_v1_15");
namespace {
dg::Engine engine(static_cast<std::uint64_t>(dg::Clock::now().time_since_epoch().count()) | 1);
static_assert(df::tile_dig_designation::No == 0 && df::tile_dig_designation::Default == 1,
    "mining/1.15 needs its reviewed designation enum mapping");
bool enabled(const char *key) {
    const char *value = std::getenv(key); return value && std::string_view(value) == "1";
}
void empty_reply(const wire::Request *in, wire::Reply *out, std::uint32_t code) {
    out->Clear(); out->set_accepted(false); out->set_failure_code(code);
    out->set_client_nonce(in->client_nonce().size() <= 64 ? in->client_nonce() : std::string());
    out->set_protocol_major(1); out->set_protocol_minor(15); out->set_bridge_generation(0);
    out->set_df_version(""); out->set_dfhack_version("");
}
void authorize(const wire::Request *in, wire::Reply *out) {
    empty_reply(in,out,3);
    dg::require(in->IsInitialized() && in->GetReflection()->GetUnknownFields(*in).empty());
    dg::require(in->client_nonce().size() >= 16 && in->client_nonce().size() <= 64);
    dg::require(in->protocol_major() == 1 && in->protocol_minor() == 15,2);
    dg::require(enabled("DFMCP_ALLOW_UNADMITTED_DIG_V1_15"),1);
    const char *configured = std::getenv("DFMCP_DIG_TOKEN");
    const std::string_view expected = configured ? configured : "";
    const auto &given = in->bearer_token();
    dg::require(expected.size() >= 32 && expected.size() <= 256 && given.size() >= 32 && given.size() <= 256,1);
    std::size_t difference = expected.size() ^ given.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0, b = i < given.size() ? given[i] : 0;
        difference |= a ^ b;
    }
    dg::require(difference == 0,1);
    dg::require(engine.generation() && engine.generation() != UINT64_MAX,5);
    const auto &v = Core::getInstance().vinfo;
    const std::string df = v ? v->getVersion() : std::string();
    const char *version = Version::dfhack_version(); dg::require(version,5);
    const std::string dfhack(version);
    dg::require(dg::utf8(df,128) && dg::utf8(dfhack,128),5);
    out->set_bridge_generation(engine.generation()); out->set_df_version(df); out->set_dfhack_version(dfhack);
}
void fortress() {
    dg::require(Core::getInstance().isWorldLoaded() && Core::getInstance().isMapLoaded() && World::isFortressMode(),4);
}
dg::Region region(const wire::Request *in) {
    dg::Region r{in->x(),in->y(),in->z(),in->width(),in->height()}; r.validate(); return r;
}
dg::Cell cell(std::uint32_t x, std::uint32_t y, std::uint32_t z) {
    dg::Cell out;
    auto *block = Maps::getTileBlock(static_cast<std::int32_t>(x),static_cast<std::int32_t>(y),static_cast<std::int32_t>(z));
    if (!block) return out;
    const auto lx=x&15, ly=y&15;
    const auto &d = block->designation[lx][ly];
    if (d.bits.hidden) { out.presence=1; return out; } // No hidden attribute reads.
    const auto tt = block->tiletype[lx][ly];
    dg::require(static_cast<std::int32_t>(tt) >= 0 && is_valid_enum_item(tt),5);
    out.presence=2; out.tiletype=static_cast<std::uint32_t>(tt);
    const auto shape = tileShape(tt);
    out.shape = shape == df::tiletype_shape::WALL ? 1 : shape == df::tiletype_shape::FLOOR ? 2 : 0;
    const auto material = tileMaterial(tt);
    out.material = material == df::tiletype_material::STONE ? 1
        : material == df::tiletype_material::MINERAL ? 2 : material == df::tiletype_material::SOIL ? 3 : 0;
    const auto &o = block->occupancy[lx][ly];
    out.flags = static_cast<std::uint8_t>(((d.bits.water_table || o.bits.heavy_aquifer) ? 1 : 0)
        | ((d.bits.feature_local || d.bits.feature_global) ? 2 : 0) | (d.bits.smooth ? 4 : 0)
        | (o.bits.dig_auto ? 8 : 0) | ((o.bits.building || o.bits.unit || o.bits.unit_grounded || o.bits.item) ? 16 : 0)
        | (tileSpecial(tt) != df::tiletype_special::NORMAL ? 32 : 0));
    out.dig=static_cast<std::uint8_t>(d.bits.dig); out.flow=static_cast<std::uint8_t>(d.bits.flow_size);
    auto other = d; other.bits.dig=df::tile_dig_designation::No; out.other_designation=other.whole;
    out.occupancy=o.whole; out.block_designated=block->flags.bits.designated;
    auto other_flags=block->flags; other_flags.bits.designated=false; out.other_block_flags=other_flags.whole;
    out.temperature1=block->temperature_1[lx][ly]; out.temperature2=block->temperature_2[lx][ly];
    return out;
}
dg::Observation capture(dg::Region r) {
    fortress(); r.validate();
    std::int32_t sx=0,sy=0,sz=0; Maps::getTileSize(sx,sy,sz);
    dg::require(sx>0 && sy>0 && sz>0 && sx<=32768 && sy<=32768 && sz<=32768,5);
    dg::require(r.x+r.width<static_cast<std::uint32_t>(sx) && r.y+r.height<static_cast<std::uint32_t>(sy)
        && r.z+1<static_cast<std::uint32_t>(sz),4);
    const auto year=static_cast<std::int64_t>(World::ReadCurrentYear()); const auto tick=World::ReadCurrentTick();
    const auto site=World::GetCurrentSiteId();
    dg::require(year>=0 && year<=UINT32_MAX && tick<403200 && site>=0 && site<=INT32_MAX,5);
    dg::Observation out; out.region=r; out.site=static_cast<std::uint32_t>(site);
    out.size_x=static_cast<std::uint32_t>(sx); out.size_y=static_cast<std::uint32_t>(sy); out.size_z=static_cast<std::uint32_t>(sz);
    out.tick=static_cast<std::uint64_t>(year)*403200+tick; out.paused=World::ReadPauseState(); out.folder=World::ReadWorldFolder();
    out.cells.reserve(r.cells()); r.halo([&](auto x,auto y,auto z){out.cells.push_back(cell(x,y,z));});
    return out;
}
void apply(dg::Region r) {
    fortress(); r.validate(); dg::require(World::ReadPauseState(),4);
    struct Target { df::map_block *block=nullptr; std::uint32_t x=0,y=0; };
    std::array<Target,64> targets{}; std::size_t count=0;
    // Resolve every pointer before writing anything. No pointer survives this
    // DFHack-owned suspension, and no map block or priority event is allocated.
    for (auto y=r.y; y<r.y+r.height; ++y) for (auto x=r.x; x<r.x+r.width; ++x) {
        auto *b=Maps::getTileBlock(static_cast<std::int32_t>(x),static_cast<std::int32_t>(y),static_cast<std::int32_t>(r.z));
        dg::require(b && !b->designation[x&15][y&15].bits.hidden
            && b->designation[x&15][y&15].bits.dig == df::tile_dig_designation::No,6);
        targets[count++]={b,x&15,y&15};
    }
    for (std::size_t i=0; i<count; ++i) {
        auto &t=targets[i];
        t.block->designation[t.x][t.y].bits.dig=df::tile_dig_designation::Default;
        t.block->flags.bits.designated=true;
    }
}
unsigned shape(const wire::Request *in) {
    return (in->has_x()?1:0) | (in->has_y()?2:0) | (in->has_z()?4:0) | (in->has_width()?8:0)
        | (in->has_height()?16:0) | (in->has_idempotency_key()?32:0) | (in->has_expected_witness()?64:0)
        | (in->has_plan_digest()?128:0) | (in->has_prepare_token()?256:0);
}
template<class F> command_result guarded(const wire::Request *in, wire::Reply *out, F f) {
    try { authorize(in,out); f(); out->set_failure_code(0); out->set_accepted(true); return CR_OK; }
    catch (const dg::Failure &e) { try { empty_reply(in,out,e.code); return CR_OK; } catch (...) {out->Clear();return CR_FAILURE;} }
    catch (...) { try { empty_reply(in,out,5); return CR_OK; } catch (...) {out->Clear();return CR_FAILURE;} }
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==0);});
}
command_result ReadDig(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==31);out->set_observation(engine.inspect(region(in),capture).encode());});
}
command_result PrepareDig(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==255);dg::require(enabled("DFMCP_DIG_ALLOW_DESIGNATE"),1);
        const auto result=engine.prepare(in->idempotency_key(),region(in),in->expected_witness(),in->plan_digest(),dg::Clock::now(),capture);
        out->set_effect_record(result.first->encode());out->set_replayed(result.second);});
}
command_result CommitDig(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==416);dg::require(enabled("DFMCP_DIG_ALLOW_DESIGNATE"),1);
        out->set_effect_record(engine.commit(in->idempotency_key(),in->plan_digest(),in->prepare_token(),dg::Clock::now(),capture,apply).encode());});
}
command_result QueryDig(color_ostream &,const wire::Request *in,wire::Reply *out) {
    return guarded(in,out,[&]{dg::require(shape(in)==160);
        if (const auto *r=engine.query(in->idempotency_key(),in->plan_digest())) out->set_effect_record(r->encode());});
}
}
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand> &) {return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream &) {engine.reset();return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event) {
    if (event==SC_MAP_LOADED || event==SC_MAP_UNLOADED || event==SC_WORLD_LOADED || event==SC_WORLD_UNLOADED) engine.reset();
    else if (event==SC_PAUSED || event==SC_UNPAUSED) engine.interrupt();
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &) {
    auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("ReadDig",ReadDig,0);
    s->addFunction("PrepareDig",PrepareDig,0);s->addFunction("CommitDig",CommitDig,0);s->addFunction("QueryDig",QueryDig,0);return s;
}
