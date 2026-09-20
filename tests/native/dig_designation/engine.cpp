#include "../../../bridge/common/dig_designation_v1_16.h"
#include <functional>
#include <iostream>
#include <stdexcept>
namespace d = dfmcp_dig_v1_16;
std::size_t checks=0;
#define CHECK(x) do { ++checks; if (!(x)) throw std::runtime_error(std::string("line ")+std::to_string(__LINE__)+": " #x); } while(false)
template<class F> void rejected(F body, unsigned code=0) {
    try { body(); } catch (const d::Failure &e) { CHECK(!code || code==e.code); return; }
    throw std::runtime_error("expected rejection");
}
d::Observation fixture(d::Region r={15,15,2,2,2}) {
    d::Observation o; o.region=r; o.size_x=64;o.size_y=64;o.size_z=8;o.site=1;o.folder="region1";o.tick=12345;o.paused=true;
    o.tiles.resize(r.cells());
    for (auto &c:o.tiles) {c.presence=2;c.tiletype=42;c.designation_other=512;c.block_other=8;c.cooldown=100;
        c.temperature1=10015;c.temperature2=10015;c.natural_wall=true;}
    return o;
}
void apply(d::Observation &o) {
    const auto &r=o.region;
    std::size_t i=0;
    for(unsigned z=r.z-1;z<=r.z+1;++z)for(unsigned y=r.y-1;y<=r.y+r.height;++y)for(unsigned x=r.x-1;x<=r.x+r.width;++x){
        auto &c=o.tiles[i++];if(c.presence!=2)continue;
        if(z==r.z&&x>=r.x&&x<r.x+r.width&&y>=r.y&&y<r.y+r.height){c.dig=1;c.priority=4000;}
        if(z==r.z&&(x/16)>=r.x/16&&(x/16)<=(r.x+r.width-1)/16
            &&(y/16)>=r.y/16&&(y/16)<=(r.y+r.height-1)/16){c.designated=true;c.cooldown=0;}
    }
}
struct Harness {
    d::Engine engine{7}; d::Observation world=fixture();
    int reads=0,writes=0;std::string key="dig-001",plan,token;
    d::Clock::time_point now=d::Clock::time_point{}+std::chrono::seconds(100);
    d::Observation read(const d::Region &r){++reads;CHECK(r==world.region);return world;}
    auto reader(){return [this](const d::Region &r){return read(r);};}
    const d::Record &prepare(bool allow=false){auto o=engine.inspect(world.region,reader());plan=d::plan_digest(world.region,allow,o.witness());
        const auto p=engine.prepare(key,world.region,allow,o.witness(),plan,now,reader());token=p.first->token;CHECK(!p.second);return *p.first;}
    const d::Record &commit(){return engine.commit(key,plan,token,now,reader(),[&](const d::Region &){
        ++writes;CHECK(engine.query(key,plan)->state==d::State::Unknown);apply(world);});}
};
void boundaries(){
    for(unsigned w=1;w<=8;++w)for(unsigned h=1;h<=8;++h){auto o=fixture({15,15,2,w,h});
        d::Engine e(7);o=e.inspect(o.region,[&](const auto&){return o;});CHECK(o.tiles.size()<301);CHECK(o.blockers(false)==0);CHECK(o.encode().size()<=d::MAX_BYTES);}
    for(unsigned v:{0u,9u,UINT32_MAX})rejected([&]{d::Region{1,1,1,v,1}.validate();});
    for(unsigned v:{0u,32767u,UINT32_MAX})rejected([&]{d::Region{1,1,v,1,1}.validate();});
    rejected([]{d::Region{32767,1,1,1,1}.validate();});
    d::Region{32766,32766,32766,1,1}.validate();
    for(const auto &s:{std::string(),std::string(129,'a'),std::string("a/b"),std::string("a\0b",3),std::string("é")})rejected([&]{d::valid_key(s);});
    d::valid_key(std::string(128,'x'));CHECK(d::utf8("é",2));CHECK(!d::utf8("\xc0\x80",2));CHECK(!d::utf8(std::string("a\0",2),2));
}
void visibility_and_policy(){
    Harness h;auto o=h.engine.inspect(h.world.region,h.reader());
    const auto check=[&](std::function<void(d::Observation&)> f,unsigned bit){auto c=o;f(c);CHECK(c.blockers(true)&bit);};
    check([](auto &c){c.paused=false;},d::Unpaused);
    check([](auto &c){c.tiles[0]=d::Cell{};},d::MissingContext);
    for(unsigned hazard:{1u,2u,4u,8u})check([&](auto &c){c.tiles[0].hazards=hazard;},d::KnownHazard);
    std::size_t target=0;o.each([&](auto i,auto x,auto y,auto z){if(o.region.target(x,y,z))target=i;});
    check([&](auto &c){c.tiles[target]=d::Cell{};c.tiles[target].presence=1;},d::UnobservedTarget);
    check([&](auto &c){c.tiles[target].natural_wall=false;},d::NotNaturalWall);
    check([&](auto &c){c.tiles[target].dig=1;},d::ExistingDesignation);
    check([&](auto &c){c.tiles[target].smooth=true;},d::ExistingDesignation);
    check([&](auto &c){c.tiles[target].occupied=true;},d::OccupiedOrJob);
    check([&](auto &c){c.tiles[target].job=true;},d::OccupiedOrJob);
    auto hidden=o;hidden.tiles[0]=d::Cell{};hidden.tiles[0].presence=1;
    CHECK(hidden.blockers(false)==d::HiddenContext);CHECK(hidden.blockers(true)==0);
    auto a=hidden.encode();hidden.tiles[0].tiletype=99;rejected([&]{hidden.encode();},5);
    hidden.tiles[0].tiletype=0;CHECK(hidden.encode()==a);
    for(unsigned tag:{3u,255u}){hidden.tiles[0].presence=tag;rejected([&]{hidden.encode();},5);}
    for(const auto &bad:{std::string(),std::string(513,'a'),std::string("x\0y",3)}){auto c=o;c.folder=bad;rejected([&]{c.encode();},5);}
    auto c=o;c.tiles.pop_back();rejected([&]{c.encode();},5);
}
void success_replay_and_seal(){
    Harness h;const auto prepared=h.prepare().encode();CHECK(h.writes==0);const auto before=h.world;
    const auto &r=h.commit();CHECK(r.state==d::State::Designated);CHECK(r.designated_count==4);CHECK(r.after_known);
    CHECK(r.receipt==r.proof());CHECK(h.writes==1);CHECK(prepared!=r.encode());const auto receipt=r.encode();
    for(int i=0;i<10;++i){CHECK(h.commit().encode()==receipt);CHECK(h.writes==1);}
    CHECK(h.engine.query(h.key,h.plan)->encode()==receipt);
    auto wrong=h.plan;wrong[0]^=1;rejected([&]{h.engine.query(h.key,wrong);},7);
    rejected([&]{h.engine.commit(h.key,h.plan,std::string(16,'x'),h.now,h.reader(),[](auto&){});},7);
    const auto o=h.engine.inspect(h.world.region,h.reader());
    rejected([&]{h.engine.prepare(h.key,h.world.region,false,o.witness(),d::plan_digest(h.world.region,false,o.witness()),h.now,h.reader());},7);
    CHECK(h.world.tiles.size()==before.tiles.size());
    for(std::size_t i=0;i<before.tiles.size();++i){const auto&a=before.tiles[i];const auto&b=h.world.tiles[i];
        CHECK(a.tiletype==b.tiletype&&a.designation_other==b.designation_other&&a.occupancy==b.occupancy&&a.block_other==b.block_other);}
    for(unsigned w=1;w<=8;++w)for(unsigned height=1;height<=8;++height){Harness v;v.world=fixture({15,15,2,w,height});v.prepare();
        CHECK(v.commit().designated_count==w*height);CHECK(v.writes==1);}
}
void stale_and_expiry(){
    using Mutate=std::function<void(d::Observation&)>;
    std::vector<Mutate> changes={[](auto&o){++o.tick;},[](auto&o){o.paused=false;},[](auto&o){++o.site;},[](auto&o){o.folder+="x";},
        [](auto&o){o.size_x=63;},[](auto&o){++o.tiles[0].tiletype;},[](auto&o){++o.tiles[0].designation_other;},
        [](auto&o){++o.tiles[0].occupancy;},[](auto&o){++o.tiles[0].priority;},[](auto&o){++o.tiles[0].cooldown;},
        [](auto&o){++o.tiles[0].block_other;},[](auto&o){++o.tiles[0].temperature1;},[](auto&o){++o.tiles[0].temperature2;},
        [](auto&o){o.tiles[0].designated=true;},[](auto&o){o.tiles[0].presence=3;}};
    for(auto change:changes){Harness h;h.prepare();change(h.world);const auto&r=h.commit();CHECK(r.state==d::State::Refused);CHECK(r.reason==d::Reason::Stale);CHECK(h.writes==0);}
    for(auto delta:{std::chrono::seconds(-1),std::chrono::seconds(60),std::chrono::seconds(61)}){Harness h;h.prepare();h.now+=delta;CHECK(h.commit().state==d::State::Refused);CHECK(h.writes==0);}
    Harness h;h.prepare();h.engine.interrupt();CHECK(h.commit().state==d::State::Refused);CHECK(h.writes==0);
    Harness replay;const auto&r=replay.prepare();auto old=r.before;const auto key=replay.key,plan=replay.plan;
    auto p=replay.engine.prepare(key,old.region,false,old.witness(),plan,replay.now+std::chrono::seconds(59),replay.reader());
    CHECK(p.second);replay.now+=std::chrono::seconds(60);CHECK(replay.commit().state==d::State::Refused);
    Harness edge;edge.now=d::Clock::time_point::max()-std::chrono::seconds(1);edge.prepare();edge.now=d::Clock::time_point::max();CHECK(edge.commit().state==d::State::Designated);
}
void uncertain_faults(){
    for(int fault=0;fault<5;++fault){Harness h;h.prepare();int called=0;
        auto r=h.engine.commit(h.key,h.plan,h.token,h.now,h.reader(),[&](const auto&){++called;
            CHECK(h.engine.query(h.key,h.plan)->state==d::State::Unknown);
            if(fault==0)throw std::runtime_error("before write");
            if(fault==1){h.world.tiles[21].dig=1;throw std::runtime_error("partial");}
            if(fault==2)return; // A no-op cannot prove nonapplication of a batch.
            apply(h.world);
            if(fault==3)h.world.tiles[0].occupancy=1;
            if(fault==4)h.world.folder.clear();
        });
        CHECK(r.state==d::State::Unknown);CHECK(!r.terminal());CHECK(!r.after_known);CHECK(called==1);CHECK(h.engine.unresolved());
        CHECK(h.engine.commit(h.key,h.plan,h.token,h.now,h.reader(),[&](auto&){++called;}).encode()==r.encode());CHECK(called==1);
        CHECK(h.engine.cancel(h.key,h.plan,h.token).encode()==r.encode());
        auto o=r.before;auto next=d::plan_digest(o.region,false,o.witness());
        rejected([&]{h.engine.prepare("another-key",o.region,false,o.witness(),next,h.now,h.reader());},8);
    }
    Harness h;h.prepare();const auto old=h.engine.query(h.key,h.plan)->before;
    auto second=h.engine.prepare("second",old.region,false,old.witness(),h.plan,h.now,h.reader());auto token=second.first->token;
    h.engine.commit(h.key,h.plan,h.token,h.now,h.reader(),[](auto&){throw std::runtime_error("unknown");});
    rejected([&]{h.engine.commit("second",h.plan,token,h.now,h.reader(),[](auto&){});},8);
    CHECK(h.engine.query("second",h.plan)->state==d::State::Prepared);
}
void cancellation_capacity_lifecycle(){
    Harness h;h.prepare();const auto&r=h.engine.cancel(h.key,h.plan,h.token);CHECK(r.state==d::State::Refused);CHECK(r.reason==d::Reason::Cancelled);
    auto saved=r.encode();CHECK(h.commit().encode()==saved);CHECK(h.writes==0);CHECK(h.engine.cancel(h.key,h.plan,h.token).encode()==saved);
    Harness many;auto o=many.engine.inspect(many.world.region,many.reader());auto plan=d::plan_digest(o.region,false,o.witness());
    for(unsigned i=0;i<d::MAX_RECORDS;++i){auto p=many.engine.prepare(std::to_string(i),o.region,false,o.witness(),plan,many.now,many.reader());CHECK(!p.second);}
    rejected([&]{many.engine.prepare("extra",o.region,false,o.witness(),plan,many.now,many.reader());},3);
    CHECK(many.engine.prepare("0",o.region,false,o.witness(),plan,many.now,many.reader()).second);
    auto generation=many.engine.generation();many.engine.reset();CHECK(many.engine.generation()==generation+1);CHECK(!many.engine.query("0",plan));
    d::Engine zero(0),last(UINT64_MAX-1);CHECK(!zero.available());CHECK(last.available());last.reset();CHECK(!last.available());last.reset();CHECK(last.generation()==UINT64_MAX);
}
std::string hex(const std::string &s){static const char*d="0123456789abcdef";std::string out;for(unsigned char c:s){out.push_back(d[c>>4]);out.push_back(d[c&15]);}return out;}
int main(int argc,char**){try{
    if(argc>1){Harness h;auto o=h.engine.inspect(h.world.region,h.reader());std::cout<<"observation "<<hex(o.encode())<<'\n';
        std::cout<<"prepared "<<hex(h.prepare().encode())<<'\n';std::cout<<"designated "<<hex(h.commit().encode())<<'\n';
        Harness r;r.prepare();std::cout<<"cancelled "<<hex(r.engine.cancel(r.key,r.plan,r.token).encode())<<'\n';return 0;}
    boundaries();visibility_and_policy();success_replay_and_seal();stale_and_expiry();uncertain_faults();cancellation_capacity_lifecycle();
    std::cout<<"{\"groups\":6,\"assertions\":"<<checks<<"}\n";return 0;
}catch(const std::exception&e){std::cerr<<e.what()<<'\n';return 1;}}
