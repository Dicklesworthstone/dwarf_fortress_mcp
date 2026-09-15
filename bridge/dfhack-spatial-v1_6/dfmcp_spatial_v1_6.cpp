#include "../common/retained_snapshot.h"
#include "../common/spatial_capture.h"
#include <cstdlib>
#include <exception>
#include <string_view>
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "DfmcpSpatialV1_6.pb.h"

using namespace DFHack;
namespace wire=dfmcp::spatial::v1_6;
namespace capture=dfmcp_spatial_capture;
DFHACK_PLUGIN("dfmcp_spatial_v1_6");
namespace {
std::uint64_t generation=static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count())|1;
dfmcp_snapshot::Cache snapshots;
capture::Bounds bounds(const wire::Request *in){return {in->max_jobs(),in->max_buildings(),in->max_items(),in->max_bytes(),
    {in->x(),in->y(),in->z()},{in->width(),in->height(),in->depth()}};}
dfmcp_snapshot::Limits limits(const wire::Request *in){return {in->max_jobs(),in->max_buildings(),in->max_items(),in->max_bytes()};}
std::string owner(const wire::Request *in){
    std::string value="dfmcp-spatial-cache-owner/1";value.push_back(0);
    capture::text(value,in->client_nonce());const auto b=bounds(in);
    for(auto n:b.origin)capture::u32(value,n);
    for(auto n:b.size)capture::u32(value,n);
    return dfmcp_snapshot::sha256(value);
}
bool authorize(const wire::Request *in,wire::Reply *out){
    out->Clear();out->set_accepted(false);out->set_failure_code(3);out->set_client_nonce("");
    out->set_protocol_major(1);out->set_protocol_minor(6);out->set_bridge_generation(0);out->set_df_version("");out->set_dfhack_version("");
    if(!bounds(in).valid()||in->client_nonce().size()<16||in->client_nonce().size()>64||
        in->page_bytes()<dfmcp_snapshot::Cache::MIN_PAGE||in->page_bytes()>dfmcp_snapshot::Cache::MAX_PAGE||
        (!in->snapshot_token().empty()&&in->snapshot_token().size()!=16))return false;
    out->set_client_nonce(in->client_nonce());
    if(in->protocol_major()!=1||in->protocol_minor()!=6){out->set_failure_code(2);return false;}
    const char *configured=std::getenv("DFMCP_SPATIAL_TOKEN");const std::string_view expected=configured?std::string_view(configured):std::string_view();
    const auto &provided=in->bearer_token();out->set_failure_code(1);
    if(expected.size()<32||expected.size()>256||provided.size()<32||provided.size()>256)return false;
    std::size_t difference=expected.size()^provided.size();
    for(std::size_t i=0;i<256;++i){const unsigned char a=i<expected.size()?expected[i]:0,b=i<provided.size()?provided[i]:0;difference|=a^b;}
    if(difference)return false;
    out->set_failure_code(5);const auto &version=Core::getInstance().vinfo;
    const auto df=version?version->getVersion():std::string(),dfhack=Version::dfhack_version();
    if(!generation||generation==std::numeric_limits<std::uint64_t>::max()||!capture::utf8(df,128)||!capture::utf8(dfhack,128))return false;
    out->set_bridge_generation(generation);out->set_df_version(df);out->set_dfhack_version(dfhack);out->set_failure_code(0);return true;
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(authorize(in,out)){
        if(!in->snapshot_token().empty()||in->offset()!=0||in->release())out->set_failure_code(3);
        else out->set_accepted(true);
    }return CR_OK;
}
command_result ReadObservation(color_ostream &,const wire::Request *in,wire::Reply *out){
    if(!authorize(in,out))return CR_OK;
    try {
        const auto now=dfmcp_snapshot::Cache::Clock::now();const auto key=owner(in);auto token=in->snapshot_token();
        if(in->release()){
            if(token.empty()||in->offset()!=0){out->set_failure_code(3);return CR_OK;}
            if(!snapshots.release(key,token,generation,limits(in),now)){out->set_failure_code(6);return CR_OK;}
            out->set_snapshot_token(token);out->set_accepted(true);return CR_OK;
        }
        if(token.empty()){
            if(in->offset()!=0){out->set_failure_code(3);return CR_OK;}
            if(!snapshots.can_capture(in->max_bytes(),now)){out->set_failure_code(3);return CR_OK;}
            std::string payload;const auto status=capture::capture(bounds(in),payload);
            if(status){out->set_failure_code(status);return CR_OK;}
            // The payload was acquired under this RPC's single suspension.
            if(!snapshots.insert(key,generation,limits(in),std::move(payload),now,token)){out->set_failure_code(3);return CR_OK;}
        }
        dfmcp_snapshot::Page page;
        if(!snapshots.page(key,token,generation,limits(in),in->offset(),in->page_bytes(),now,page)){out->set_failure_code(6);return CR_OK;}
        out->set_observation(page.bytes);out->set_snapshot_token(page.token);out->set_page_offset(page.offset);
        out->set_total_bytes(page.total);out->set_payload_sha256(page.digest);out->set_complete(page.complete);out->set_accepted(true);
    }catch(const std::exception &){
        // Never expose partial fields after an allocation/serialization failure.
        out->Clear();out->set_accepted(false);out->set_failure_code(3);out->set_client_nonce(in->client_nonce());
        out->set_protocol_major(1);out->set_protocol_minor(6);out->set_bridge_generation(0);out->set_df_version("");out->set_dfhack_version("");
    }
    return CR_OK;
}
}
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand> &){return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream &){snapshots.clear();return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event){
    if(event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED){snapshots.clear();if(generation!=std::numeric_limits<std::uint64_t>::max())++generation;}
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &){auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("ReadObservation",ReadObservation,0);return s;}
