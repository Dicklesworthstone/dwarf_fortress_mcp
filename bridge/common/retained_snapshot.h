#pragma once

#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <limits>
#include <map>
#include <string>
#include <utility>

// No native pointers survive capture. The caller serializes under the game
// suspension, then this bounded cache owns only immutable bytes and metadata.
namespace dfmcp_snapshot {

inline std::string sha256(const std::string &input)
{
    constexpr std::uint32_t k[] = {
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
    };
    std::array<std::uint32_t,8> h={0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};
    const auto rotate=[](std::uint32_t v,unsigned n){return (v>>n)|(v<<(32-n));};
    const std::size_t blocks=input.size()/64+1+(input.size()%64>=56 ? 1 : 0);
    const std::uint64_t bits=static_cast<std::uint64_t>(input.size())*8;
    for(std::size_t block=0;block<blocks;++block) {
        std::uint32_t w[64]={};
        for(std::size_t i=0;i<64;++i) {
            const auto offset=block*64+i;
            unsigned char byte=offset<input.size() ? static_cast<unsigned char>(input[offset]) : offset==input.size() ? 128 : 0;
            if(block==blocks-1 && i>=56) byte=static_cast<unsigned char>(bits>>((63-i)*8));
            w[i/4]|=std::uint32_t{byte}<<(24-8*(i%4));
        }
        for(unsigned i=16;i<64;++i) {
            const auto a=rotate(w[i-15],7)^rotate(w[i-15],18)^(w[i-15]>>3);
            const auto b=rotate(w[i-2],17)^rotate(w[i-2],19)^(w[i-2]>>10);
            w[i]=w[i-16]+a+w[i-7]+b;
        }
        auto a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],v=h[7];
        for(unsigned i=0;i<64;++i) {
            const auto t=v+(rotate(e,6)^rotate(e,11)^rotate(e,25))+((e&f)^(~e&g))+k[i]+w[i];
            const auto u=(rotate(a,2)^rotate(a,13)^rotate(a,22))+((a&b)^(a&c)^(b&c));
            v=g;g=f;f=e;e=d+t;d=c;c=b;b=a;a=t+u;
        }
        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=v;
    }
    std::string out;
    for(auto word:h) for(int shift=24;shift>=0;shift-=8) out.push_back(static_cast<char>((word>>shift)&255));
    return out;
}

struct Limits {
    std::uint32_t jobs,buildings,items,bytes;
    bool operator==(const Limits &v) const { return jobs==v.jobs && buildings==v.buildings && items==v.items && bytes==v.bytes; }
};
struct Page {
    std::string token,digest,bytes;
    std::uint64_t generation=0,total=0,offset=0;
    bool complete=false;
};

class Cache {
public:
    using Clock=std::chrono::steady_clock;
    static constexpr std::size_t MAX_BYTES=32*1024*1024;
    static constexpr std::size_t MAX_CAPTURE_BYTES=16*1024*1024;
    static constexpr std::size_t MAX_CAPTURES=4;
    static constexpr std::size_t MIN_PAGE=16*1024,MAX_PAGE=256*1024;
    static constexpr std::chrono::seconds LIFETIME{120};

    bool can_capture(std::size_t maximum,Clock::time_point now) {
        expire(now);
        return maximum>0 && maximum<=MAX_CAPTURE_BYTES && entries.size()<MAX_CAPTURES && maximum<=MAX_BYTES-retained;
    }
    // Caller checks capacity BEFORE serializing to bound peak retained+capture memory.
    bool insert(const std::string &owner,std::uint64_t generation,Limits limits,
                std::string payload,Clock::time_point now,std::string &token) {
        if(owner.size()<16 || owner.size()>64 || !generation || payload.empty() || payload.size()>limits.bytes ||
            !can_capture(limits.bytes,now) || serial==std::numeric_limits<std::uint64_t>::max()) return false;
        std::string id;
        for(auto word:{generation,++serial}) for(int shift=56;shift>=0;shift-=8) id.push_back(static_cast<char>((word>>shift)&255));
        Entry entry{owner,generation,limits,std::move(payload),{},now+LIFETIME};
        entry.digest=sha256(entry.payload);
        auto result=entries.emplace(id,std::move(entry));
        if(!result.second) return false;
        retained+=result.first->second.payload.size();
        // Do not allocate after publishing the cache entry. A throwing token
        // copy would strand a retained capture whose handle was never returned.
        token.swap(id);
        return true;
    }
    bool page(const std::string &owner,const std::string &token,std::uint64_t generation,Limits limits,
              std::uint64_t offset,std::size_t width,Clock::time_point now,Page &out) {
        expire(now);
        if(token.size()!=16 || width<MIN_PAGE || width>MAX_PAGE) return false;
        const auto found=entries.find(token);
        if(found==entries.end()) return false;
        const auto &entry=found->second;
        if(entry.owner!=owner || entry.generation!=generation || !(entry.limits==limits) || offset>=entry.payload.size()) return false;
        const auto size=std::min(width,entry.payload.size()-static_cast<std::size_t>(offset));
        out=Page{token,entry.digest,entry.payload.substr(static_cast<std::size_t>(offset),size),generation,
            static_cast<std::uint64_t>(entry.payload.size()),offset,offset+size==entry.payload.size()};
        return true;
    }
    bool release(const std::string &owner,const std::string &token,std::uint64_t generation,Limits limits,Clock::time_point now) {
        expire(now);
        const auto found=entries.find(token);
        if(found==entries.end() || found->second.owner!=owner || found->second.generation!=generation || !(found->second.limits==limits)) return false;
        retained-=found->second.payload.size();entries.erase(found);return true;
    }
    void clear() { entries.clear();retained=0; } // Do not reuse the monotone token counter.
    std::size_t bytes() const {return retained;}
    std::size_t count() const {return entries.size();}
private:
    struct Entry {
        std::string owner;
        std::uint64_t generation;
        Limits limits;
        std::string payload,digest;
        Clock::time_point expires;
    };
    std::map<std::string,Entry> entries;
    std::size_t retained=0;
    std::uint64_t serial=0;
    void expire(Clock::time_point now) {
        for(auto it=entries.begin();it!=entries.end();) {
            if(now>=it->second.expires) {retained-=it->second.payload.size();it=entries.erase(it);}
            else ++it;
        }
    }
};
}
