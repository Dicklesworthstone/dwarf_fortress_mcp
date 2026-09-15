#pragma once
#include <cstdint>
#include <string>
#include "citizen_capture.h"
#include "spatial_capture.h"

// Composite 1.8 capture. All three components are acquired during one caller RPC
// suspension. Component codecs remain data-only; no independently timed worlds
// are merged here.
namespace dfmcp_spatial_citizen_capture {
namespace spatial=dfmcp_spatial_capture;
namespace citizens=dfmcp_citizen_capture;
struct Bounds {
    spatial::Bounds spatial_bounds;
    std::uint32_t max_citizens;
    bool valid() const {return spatial_bounds.valid()&&max_citizens>0&&max_citizens<=4096;}
};
inline std::uint32_t capture(const Bounds &bounds,std::string &out){
    if(!bounds.valid())return 3;
    std::string base,units;
    auto status=spatial::capture(bounds.spatial_bounds,base);if(status)return status;
    status=citizens::capture(bounds.max_citizens,bounds.spatial_bounds.bytes,units);if(status)return status;
    const auto total=base.size()+units.size()+16;
    if(total>bounds.spatial_bounds.bytes)return 3;
    out.assign("DFMS1800",8);
    spatial::u32(out,static_cast<std::uint32_t>(base.size()));out+=base;
    spatial::u32(out,static_cast<std::uint32_t>(units.size()));out+=units;
    return out.size()>bounds.spatial_bounds.bytes?3:0;
}
}
