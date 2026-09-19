#include "../common/retained_snapshot.h"
#include "../common/spatial_citizen_capture.h"
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <exception>
#include <limits>
#include <string>
#include <string_view>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "DfmcpSpatialV1_8.pb.h"

using namespace DFHack;
namespace wire=dfmcp::spatial::v1_8;
namespace capture=dfmcp_spatial_citizen_capture;
namespace base=dfmcp_spatial_capture;
DFHACK_PLUGIN("dfmcp_spatial_v1_8");
namespace {
std::uint64_t generation=static_cast<std::uint64_t>(std::chrono::steady_clock::now().time_since_epoch().count())|1;
dfmcp_snapshot::Cache snapshots;
base::Bounds spatial_bounds(const wire::Request *in){return {in->max_jobs(),in->max_buildings(),in->max_items(),in->max_bytes(),
    {in->x(),in->y(),in->z()},{in->width(),in->height(),in->depth()}};}
capture::Bounds bounds(const wire::Request *in){return {spatial_bounds(in),in->max_citizens()};}
dfmcp_snapshot::Limits limits(const wire::Request *in){return {in->max_jobs(),in->max_buildings(),in->max_items(),in->max_bytes()};}
std::string owner(const wire::Request *in){
    std::string value="dfmcp-spatial-citizen-cache-owner/1";value.push_back('\0');base::text(value,in->client_nonce());
    const auto b=bounds(in);for(auto n:b.spatial_bounds.origin)base::u32(value,n);for(auto n:b.spatial_bounds.size)base::u32(value,n);
    base::u32(value,b.max_citizens);return dfmcp_snapshot::sha256(value);
}
bool authorize(const wire::Request *in,wire::Reply *out){
    out->Clear();out->set_accepted(false);out->set_failure_code(3);out->set_client_nonce("");
    out->set_protocol_major(1);out->set_protocol_minor(8);out->set_bridge_generation(0);out->set_df_version("");out->set_dfhack_version("");
    if(!bounds(in).valid()||in->client_nonce().size()<16||in->client_nonce().size()>64||
        in->page_bytes()<dfmcp_snapshot::Cache::MIN_PAGE||in->page_bytes()>dfmcp_snapshot::Cache::MAX_PAGE||
        (!in->snapshot_token().empty()&&in->snapshot_token().size()!=16))return false;
    out->set_client_nonce(in->client_nonce());
    if(in->protocol_major()!=1||in->protocol_minor()!=8){out->set_failure_code(2);return false;}
    const char *configured=std::getenv("DFMCP_SPATIAL_CITIZEN_TOKEN");const std::string_view expected=configured?std::string_view(configured):std::string_view();
    const auto &provided=in->bearer_token();out->set_failure_code(1);
    if(expected.size()<32||expected.size()>256||provided.size()<32||provided.size()>256)return false;
    std::size_t difference=expected.size()^provided.size();for(std::size_t i=0;i<256;++i){const unsigned char a=i<expected.size()?expected[i]:0,b=i<provided.size()?provided[i]:0;difference|=a^b;}if(difference)return false;
    out->set_failure_code(5);const auto &version=Core::getInstance().vinfo;
    const std::string df=version?version->getVersion():std::string();
    // DFHack declares const char*, not std::string. Keep the pointer separate
    // and reject a missing version instead of constructing a string from null.
    const char *native_version=Version::dfhack_version();
    const std::string dfhack=native_version?native_version:"";
    if(!generation||generation==std::numeric_limits<std::uint64_t>::max()||!base::utf8(df,128)||!base::utf8(dfhack,128))return false;
    out->set_bridge_generation(generation);out->set_df_version(df);out->set_dfhack_version(dfhack);out->set_failure_code(0);return true;
}
// Exception text is never returned: it may contain native/private data. Clear
// every payload/identity field, including fields already set by a failed reply.
command_result exception_reply(const wire::Request *in,wire::Reply *out){
    try{
        out->Clear();out->set_accepted(false);out->set_failure_code(5);
        const auto &nonce=in->client_nonce();
        out->set_client_nonce(nonce.size()>=16&&nonce.size()<=64?nonce:std::string());
        out->set_protocol_major(1);out->set_protocol_minor(8);out->set_bridge_generation(0);
        out->set_df_version("");out->set_dfhack_version("");
        return CR_OK;
    }catch(...){
        // If even an error envelope cannot be allocated, let DFHack report the
        // RPC failure. A partially accepted response must not escape instead.
        out->Clear();return CR_FAILURE;
    }
}
command_result Handshake(color_ostream &,const wire::Request *in,wire::Reply *out){
    try{
        if(authorize(in,out)){
            if(!in->snapshot_token().empty()||in->offset()!=0||in->release())out->set_failure_code(3);
            else out->set_accepted(true);
        }
    }catch(...){return exception_reply(in,out);}
    return CR_OK;
}
command_result ReadObservation(color_ostream &,const wire::Request *in,wire::Reply *out){
    std::string key,token;bool inserted=false;
    dfmcp_snapshot::Cache::Clock::time_point now;
    try{
        if(!authorize(in,out))return CR_OK;
        now=dfmcp_snapshot::Cache::Clock::now();key=owner(in);token=in->snapshot_token();
        if(in->release()){
            if(token.empty()||in->offset()!=0){out->set_failure_code(3);return CR_OK;}
            if(!snapshots.release(key,token,generation,limits(in),now)){out->set_failure_code(6);return CR_OK;}
            out->set_snapshot_token(token);out->set_accepted(true);return CR_OK;
        }
        if(token.empty()){
            if(in->offset()!=0||!snapshots.can_capture(in->max_bytes(),now)){out->set_failure_code(3);return CR_OK;}
            std::string payload;const auto status=capture::capture(bounds(in),payload);if(status){out->set_failure_code(status);return CR_OK;}
            if(!snapshots.insert(key,generation,limits(in),std::move(payload),now,token)){out->set_failure_code(3);return CR_OK;}
            inserted=true;
        }
        dfmcp_snapshot::Page page;
        if(!snapshots.page(key,token,generation,limits(in),in->offset(),in->page_bytes(),now,page)){
            if(inserted)snapshots.release(key,token,generation,limits(in),now);
            out->set_failure_code(6);return CR_OK;
        }
        out->set_observation(page.bytes);out->set_snapshot_token(page.token);out->set_page_offset(page.offset);out->set_total_bytes(page.total);
        out->set_payload_sha256(page.digest);out->set_complete(page.complete);out->set_accepted(true);
    }catch(...){
        // Only this call's newly captured bytes are unpublished. A failed later
        // page must retain its previously acknowledged immutable capture token.
        if(inserted)snapshots.release(key,token,generation,limits(in),now);
        return exception_reply(in,out);
    }
    return CR_OK;
}
}
DFhackCExport command_result plugin_init(color_ostream &,std::vector<PluginCommand>&){return CR_OK;}
DFhackCExport command_result plugin_shutdown(color_ostream &){snapshots.clear();return CR_OK;}
DFhackCExport command_result plugin_onstatechange(color_ostream &,state_change_event event){if(event==SC_WORLD_LOADED||event==SC_WORLD_UNLOADED||event==SC_MAP_LOADED||event==SC_MAP_UNLOADED){snapshots.clear();if(generation!=std::numeric_limits<std::uint64_t>::max())++generation;}return CR_OK;}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &){auto *s=new RPCService();s->addFunction("Handshake",Handshake,0);s->addFunction("ReadObservation",ReadObservation,0);return s;}
