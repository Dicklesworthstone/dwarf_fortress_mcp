#include "../retained_snapshot.h"
#include <cassert>
#include <iostream>
using namespace dfmcp_snapshot;
std::string hex(const std::string &s) {std::string out;for(unsigned char c:s){out.push_back("0123456789abcdef"[c>>4]);out.push_back("0123456789abcdef"[c&15]);}return out;}
int main() {
    for(std::size_t n: {0u,1u,55u,56u,63u,64u,65u,127u,128u,16384u,1048576u}) std::cout<<n<<":"<<hex(sha256(std::string(n,'x')))<<"\n";
    Cache cache;auto now=Cache::Clock::time_point{};std::string owner(16,'n'),token;
    Limits limits{4096,4096,65536,1024*1024};std::string data(16385,'a');
    assert(cache.insert(owner,7,limits,data,now,token));data[0]='z';
    Page p;assert(cache.page(owner,token,7,limits,0,16384,now,p));assert(p.bytes[0]=='a'&&!p.complete);
    Page same;assert(cache.page(owner,token,7,limits,0,16384,now,same));assert(same.bytes==p.bytes&&same.digest==p.digest);
    assert(cache.page(owner,token,7,limits,16384,16384,now,p));assert(p.complete&&p.bytes.size()==1);
    assert(!cache.page(std::string(16,'x'),token,7,limits,0,16384,now,p));
    assert(!cache.page(owner,token,8,limits,0,16384,now,p));
    assert(!cache.page(owner,token,7,Limits{4096,4096,1,1024*1024},0,16384,now,p));
    assert(!cache.page(owner,token,7,limits,16385,16384,now,p));assert(!cache.page(owner,token,7,limits,0,1,now,p));
    assert(!cache.release(std::string(16,'x'),token,7,limits,now));
    assert(cache.release(owner,token,7,limits,now));assert(cache.bytes()==0&&cache.count()==0);
    assert(!cache.page(owner,token,7,limits,0,16384,now,p));
    auto old=token;assert(cache.insert(owner,7,limits,data,now,token));assert(token!=old);
    assert(!cache.page(owner,token,7,limits,0,16384,now+Cache::LIFETIME,p));assert(cache.bytes()==0);
    for(int i=0;i<4;++i)assert(cache.insert(owner,7,limits,"x",now,token));
    assert(!cache.can_capture(1024,now));assert(!cache.insert(owner,7,limits,"x",now,token));
    cache.clear();assert(cache.bytes()==0&&cache.count()==0);
    Limits large{4096,4096,65536,16*1024*1024};
    assert(cache.insert(owner,7,large,std::string(16*1024*1024,'x'),now,token));
    assert(cache.insert(owner,7,large,std::string(16*1024*1024,'x'),now,token));
    assert(!cache.can_capture(1,now));
    std::cout<<"cache checks passed\n";
}
