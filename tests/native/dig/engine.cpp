#include "../../../bridge/common/dig_designation.h"
#include <iostream>
#include <stdexcept>
#include <functional>
using namespace dfmcp_dig;
static unsigned assertions = 0;
#define CHECK(x) do { ++assertions; if (!(x)) throw std::runtime_error("CHECK failed: " #x); } while (false)
template<class F> void refuses(F f, unsigned code = 0) {
    bool caught = false;
    try { f(); } catch (const Failure &e) { caught = true; CHECK(!code || e.code == code); }
    CHECK(caught);
}
static const Region region{15,15,2,2,2};
static const auto now = Clock::time_point{} + std::chrono::seconds(1000);
Observation fixture(Region r = region) {
    Observation out; out.generation = 7; out.region = r; out.tick = 12345; out.site = 1;
    out.size_x = 64; out.size_y = 64; out.size_z = 8; out.folder = "region1"; out.paused = true;
    Cell c; c.presence = 2; c.shape = 1; c.material = 1; c.tiletype = 42;
    c.temperature1 = c.temperature2 = 10015;
    out.cells.assign(r.cells(),c); return out;
}
struct World {
    Observation value = fixture();
    unsigned reads = 0, writes = 0;
    Observation read(Region r) { ++reads; CHECK(r.encode() == value.region.encode()); return value; }
    void apply(Region r) {
        ++writes; std::size_t i = 0;
        r.halo([&](auto x, auto y, auto z) {
            auto &c = value.cells[i++];
            if (r.target(x,y,z)) c.dig = 1;
            if (r.touched_block(x,y,z)) c.block_designated = true;
        });
    }
};
const Record &prepare(Engine &e, World &w, const std::string &key = "mine-001") {
    auto read = [&](Region r) { return w.read(r); };
    const auto o = e.inspect(w.value.region,read);
    return *e.prepare(key,o.region,o.witness(),plan_digest(o.witness()),now,read).first;
}
Record commit(Engine &e, World &w, const Record &p, Clock::time_point when = now) {
    return e.commit(p.key,p.plan,p.token,when,[&](Region r){ return w.read(r); },[&](Region r){w.apply(r);});
}
std::string hex(const std::string &bytes) {
    const char *digits = "0123456789abcdef"; std::string out;
    for (unsigned char c : bytes) { out += digits[c>>4]; out += digits[c&15]; } return out;
}
void shape_and_eligibility() {
    for (unsigned w = 0; w <= 9; ++w) for (unsigned h = 0; h <= 9; ++h) {
        Region r{15,15,2,w,h};
        if (w >= 1 && w <= 8 && h >= 1 && h <= 8) { CHECK(fixture(r).eligible()); CHECK(r.cells() <= 300); }
        else refuses([&]{ r.validate(); });
    }
    for (auto r : {Region{0,1,1,1,1}, Region{1,0,1,1,1}, Region{1,1,0,1,1},
        Region{UINT32_MAX,1,1,1,1}, Region{32766,1,1,2,1}}) refuses([&]{r.validate();});
    auto bad = fixture(); bad.size_x = 17; refuses([&]{bad.encode();},5);
    bad = fixture(); bad.cells.pop_back(); refuses([&]{bad.encode();},5);
    for (const auto &s : {std::string(), std::string("a\0b",3), std::string("\xc0\x80",2),
        std::string("\xed\xa0\x80",3), std::string(513,'a')}) {
        bad = fixture(); bad.folder = s; refuses([&]{bad.encode();},5);
    }
    for (std::size_t i = 0; i < fixture().cells.size(); ++i) {
        for (unsigned kind = 0; kind < 7; ++kind) {
            bad = fixture(); auto &c = bad.cells[i];
            switch (kind) {
                case 0: c = Cell{}; break;
                case 1: c = Cell{}; c.presence = 1; break;
                case 2: c.flags = 1; break;
                case 3: c.flags = 2; break;
                case 4: c.flags = 8; break;
                case 5: c.flow = 1; break;
                default: c.temperature2 = 10075;
            }
            CHECK(!bad.eligible());
        }
    }
    bad = fixture(); bad.cells[0].presence = 1; refuses([&]{bad.encode();},5);
    auto hidden = fixture(); hidden.cells[0] = Cell{}; hidden.cells[0].presence = 1;
    CHECK(hidden.encode().size() + 26 == fixture().encode().size());
    for (unsigned flag : {4u,16u,32u}) {
        bad = fixture(); bad.cells[21].flags = static_cast<std::uint8_t>(flag); CHECK(!bad.eligible());
    }
}
void exact_success_and_replay() {
    for (unsigned width = 1; width <= 8; ++width) for (unsigned height = 1; height <= 8; ++height) {
        World w; w.value = fixture(Region{15,15,2,width,height}); Engine e(7);
        const auto original = e.inspect(w.value.region,[&](Region r){return w.read(r);});
        const auto p = prepare(e,w); const auto receipt = commit(e,w,p);
        CHECK(receipt.state == State::Designated); CHECK(w.writes == 1);
        CHECK(receipt.designated_tiles == width*height);
        CHECK(receipt.after_witness == original.expected_after().witness());
        CHECK(receipt.receipt == receipt.proof());
        const auto reads = w.reads;
        CHECK(commit(e,w,p).encode() == receipt.encode()); CHECK(w.writes == 1); CHECK(w.reads == reads);
        CHECK(e.query(p.key,p.plan)->encode() == receipt.encode());
        auto replay = e.prepare(p.key,p.before.region,p.witness,p.plan,now+LIFETIME,
            [&](Region r){return w.read(r);});
        CHECK(replay.second); CHECK(replay.first->encode() == receipt.encode()); CHECK(w.reads == reads);
    }
}
void stale_witness_matrix() {
    for (std::size_t i = 0; i < fixture().cells.size(); ++i) for (unsigned kind = 0; kind < 9; ++kind) {
        World w; Engine e(7); const auto p = prepare(e,w); auto &c = w.value.cells[i];
        switch (kind) {
            case 0: c.other_designation ^= 0x8000; break;
            case 1: c.occupancy ^= 0x4000; break;
            case 2: c.other_block_flags ^= 0x80; break;
            case 3: ++c.temperature1; break;
            case 4: ++c.temperature2; break;
            case 5: ++c.tiletype; break;
            case 6: c.block_designated = true; break;
            case 7: c.material = 2; break;
            default: c.dig = 1;
        }
        auto out = commit(e,w,p); CHECK(out.state == State::Refused); CHECK(w.writes == 0);
        CHECK(out.receipt == out.proof());
    }
    for (unsigned field = 0; field < 6; ++field) {
        World w; Engine e(7); const auto p = prepare(e,w);
        switch (field) {
            case 0: ++w.value.tick; break; case 1: ++w.value.site; break;
            case 2: w.value.folder += "x"; break; case 3: w.value.paused = false; break;
            case 4: ++w.value.size_x; break; default: e.interrupt();
        }
        CHECK(commit(e,w,p).state == State::Refused); CHECK(w.writes == 0);
    }
}
void uncertainty_and_readback() {
    for (unsigned failure = 0; failure <= 7; ++failure) {
        World w; Engine e(7); const auto p = prepare(e,w); const auto q = prepare(e,w,"other");
        unsigned reads = 0;
        auto read = [&](Region r) { if (failure == 0 && ++reads == 2) throw std::bad_alloc(); return w.read(r); };
        auto apply = [&](Region r) {
            if (failure == 1) { ++w.writes; throw std::bad_alloc(); }
            w.apply(r);
            if (failure == 2) throw std::bad_alloc();
            if (failure == 3) ++w.value.cells[0].other_designation;
            if (failure == 4) w.value.cells[21].dig = 0;
            if (failure == 5) w.value.cells[21].block_designated = false;
            if (failure == 6) ++w.value.tick;
            if (failure == 7) w.value.paused = false;
        };
        const auto out = e.commit(p.key,p.plan,p.token,now,read,apply);
        CHECK(out.state == State::Unknown); CHECK(e.unresolved()); CHECK(w.writes == 1);
        CHECK(out.designated_tiles == 0 && out.after_witness == std::string(32,'\0') && out.receipt == std::string(32,'\0'));
        CHECK(commit(e,w,p).state == State::Unknown); CHECK(w.writes == 1);
        refuses([&]{ commit(e,w,q); },8); CHECK(w.writes == 1);
        w.value = fixture(); refuses([&]{prepare(e,w,"new-key");},8);
    }
    // A partial batch is retained as unknown rather than rolled back or replayed.
    for (unsigned prefix = 0; prefix <= 4; ++prefix) {
        World w; Engine e(7); const auto p = prepare(e,w);
        e.commit(p.key,p.plan,p.token,now,[&](Region r){return w.read(r);},[&](Region r){
            ++w.writes; unsigned count = 0; std::size_t i = 0;
            r.halo([&](auto x,auto y,auto z){ auto &c=w.value.cells[i++];
                if (r.target(x,y,z)) { if (count++ == prefix) throw std::bad_alloc(); c.dig=1; }
            }); throw std::bad_alloc();
        });
        CHECK(e.query(p.key,p.plan)->state == State::Unknown); CHECK(commit(e,w,p).state == State::Unknown); CHECK(w.writes == 1);
    }
}
void identity_lifetime_and_capacity() {
    for (const auto delta : {-1,0,59,60,61}) {
        World w; Engine e(7); const auto p = prepare(e,w);
        auto replay = e.prepare(p.key,p.before.region,p.witness,p.plan,now+std::chrono::seconds(59),[&](Region r){return w.read(r);});
        CHECK(replay.second); CHECK(replay.first->created == now);
        CHECK(commit(e,w,p,now+std::chrono::seconds(delta)).state == (delta >= 0 && delta < 60 ? State::Designated : State::Refused));
    }
    World w; Engine e(7); const auto p = prepare(e,w);
    refuses([&]{e.query(p.key,std::string(32,'x'));},7);
    refuses([&]{e.commit(p.key,p.plan,std::string(16,'x'),now,[&](Region r){return w.read(r);},[&](Region r){w.apply(r);});},7);
    auto r = region; ++r.x;
    refuses([&]{e.prepare(p.key,r,p.witness,p.plan,now,[&](Region q){return w.read(q);});},7);
    for (const auto &key : {std::string(),std::string("a b"),std::string("../x"),std::string(129,'a')}) refuses([&]{valid_key(key);});
    valid_key(std::string(128,'a')); e.reset(); CHECK(!e.query(p.key,p.plan));
    refuses([&]{commit(e,w,p);},7); CHECK(w.writes == 0);
    Engine cap(7); for (unsigned i=0;i<MAX_RECORDS;++i) prepare(cap,w,"key-"+std::to_string(i));
    CHECK(cap.size()==MAX_RECORDS); refuses([&]{prepare(cap,w,"overflow");},5); CHECK(w.writes == 0);
    for (auto generation : {std::uint64_t{0},std::uint64_t{UINT64_MAX}}) { Engine broken(generation); refuses([&]{prepare(broken,w);},5); }
    Engine almost(7,UINT64_MAX-2); const auto last = prepare(almost,w); CHECK(commit(almost,w,last).state==State::Designated);
    almost.interrupt(); CHECK(!almost.available()); almost.reset(); CHECK(!almost.available());
    Engine last_generation(UINT64_MAX-1); last_generation.reset(); CHECK(!last_generation.available());
}
int main() {
    try {
        shape_and_eligibility(); exact_success_and_replay(); stale_witness_matrix(); uncertainty_and_readback(); identity_lifetime_and_capacity();
        World w; Engine e(7); const auto p=prepare(e,w); const auto result=commit(e,w,p);
        World refused_world; Engine refused_engine(7); auto refused=prepare(refused_engine,refused_world);
        refused=commit(refused_engine,refused_world,refused,now+LIFETIME);
        std::cout << "{\"groups\":5,\"assertions\":" << assertions << ",\"vectors\":{\"observation\":\""
            << hex(p.before.encode()) << "\",\"prepared\":\"" << hex(p.encode()) << "\",\"designated\":\""
            << hex(result.encode()) << "\",\"refused\":\"" << hex(refused.encode()) << "\"}}\n";
        return 0;
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
