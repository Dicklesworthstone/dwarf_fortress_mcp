#pragma once
#include "retained_snapshot.h"
#include <exception>
#include <vector>

// One fixed effect: request ordinary mining of a small visible natural-wall
// rectangle. The caller owns a DFHack suspension for every capture/write callback.
// This is designation readback, not excavation, structural safety or game truth.
namespace dfmcp_dig {
using Clock = std::chrono::steady_clock;
constexpr std::size_t MAX_RECORDS = 128, MAX_OBSERVATION = 16 * 1024;
constexpr std::chrono::seconds LIFETIME{60};
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t c) : code(c) {}
    const char *what() const noexcept override { return "bounded mining refused"; }
};
inline void require(bool ok, std::uint32_t code = 3) { if (!ok) throw Failure(code); }
inline void u32(std::string &out, std::uint32_t n) {
    for (int shift = 24; shift >= 0; shift -= 8) out.push_back(static_cast<char>(n >> shift));
}
inline void u64(std::string &out, std::uint64_t n) {
    for (int shift = 56; shift >= 0; shift -= 8) out.push_back(static_cast<char>(n >> shift));
}
inline void text(std::string &out, const std::string &value) {
    require(value.size() <= UINT16_MAX);
    out.push_back(static_cast<char>(value.size() >> 8));
    out.push_back(static_cast<char>(value.size())); out += value;
}
inline std::string hash(const char *domain, const std::string &bytes) {
    std::string framed(domain); framed.push_back('\0'); framed += bytes;
    return dfmcp_snapshot::sha256(framed);
}
inline bool utf8(const std::string &value, std::size_t maximum) {
    if (value.empty() || value.size() > maximum) return false;
    for (std::size_t i = 0; i < value.size();) {
        const auto a = static_cast<unsigned char>(value[i]);
        if (!a) return false;
        if (a < 128) { ++i; continue; }
        const std::size_t n = a >= 0xc2 && a <= 0xdf ? 2 : a >= 0xe0 && a <= 0xef ? 3 : a >= 0xf0 && a <= 0xf4 ? 4 : 0;
        if (!n || value.size() - i < n) return false;
        for (std::size_t j = 1; j < n; ++j)
            if ((static_cast<unsigned char>(value[i+j]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(value[i+1]);
        if ((a == 0xe0 && b < 0xa0) || (a == 0xed && b >= 0xa0)
            || (a == 0xf0 && b < 0x90) || (a == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void valid_key(const std::string &key) {
    require(!key.empty() && key.size() <= 128);
    for (unsigned char c : key)
        require((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
            || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-');
}
struct Region {
    std::uint32_t x = 0, y = 0, z = 0, width = 0, height = 0;
    void validate() const {
        require(x > 0 && y > 0 && z > 0 && x < 32767 && y < 32767 && z < 32767
            && width >= 1 && width <= 8 && height >= 1 && height <= 8
            && width < 32768 - x && height < 32768 - y);
    }
    std::uint32_t cells() const { return (width + 2) * (height + 2) * 3; }
    std::string encode() const {
        validate(); std::string out;
        for (auto n : {x,y,z,width,height}) u32(out,n);
        return out;
    }
    template<class F> void halo(F f) const {
        validate();
        for (auto iz = z-1; iz <= z+1; ++iz)
        for (auto iy = y-1; iy <= y+height; ++iy)
        for (auto ix = x-1; ix <= x+width; ++ix) f(ix,iy,iz);
    }
    bool target(std::uint32_t ix, std::uint32_t iy, std::uint32_t iz) const {
        return iz == z && ix >= x && ix < x+width && iy >= y && iy < y+height;
    }
    bool touched_block(std::uint32_t ix, std::uint32_t iy, std::uint32_t iz) const {
        return iz == z && ix/16 >= x/16 && ix/16 <= (x+width-1)/16
            && iy/16 >= y/16 && iy/16 <= (y+height-1)/16;
    }
};
// flags: aquifer=1, feature=2, smoothing=4, auto-dig=8,
// occupied (building/unit/item)=16, non-normal tile special=32.
struct Cell {
    std::uint8_t presence = 0, shape = 0, material = 0, dig = 0, flags = 0, flow = 0;
    bool block_designated = false;
    std::uint32_t tiletype = 0, other_designation = 0, occupancy = 0, other_block_flags = 0;
    std::uint16_t temperature1 = 0, temperature2 = 0;
    void append(std::string &out) const {
        require(presence <= 2 && shape <= 2 && material <= 3 && dig <= 7 && flags <= 63 && flow <= 7, 5);
        out.push_back(static_cast<char>(presence));
        if (presence != 2) {
            require(!shape && !material && !dig && !flags && !flow && !block_designated
                && !tiletype && !other_designation && !occupancy && !other_block_flags
                && !temperature1 && !temperature2, 5);
            return; // Hidden/unallocated attributes never cross the bridge.
        }
        require(tiletype <= INT32_MAX, 5);
        for (auto n : {shape,material,dig,flags,flow}) out.push_back(static_cast<char>(n));
        out.push_back(block_designated ? 1 : 0);
        for (auto n : {tiletype,other_designation,occupancy,other_block_flags}) u32(out,n);
        for (auto n : {temperature1,temperature2}) {
            out.push_back(static_cast<char>(n >> 8)); out.push_back(static_cast<char>(n));
        }
    }
};
struct Observation {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0, size_x = 0, size_y = 0, size_z = 0;
    Region region;
    std::string folder;
    bool paused = false;
    std::vector<Cell> cells;
    std::string encode() const {
        region.validate();
        require(generation && generation != UINT64_MAX && sequence != UINT64_MAX
            && tick <= std::uint64_t{UINT32_MAX} * 403200 + 403199 && site <= INT32_MAX
            && size_x <= 32768 && size_y <= 32768 && size_z <= 32768
            && region.x + region.width < size_x && region.y + region.height < size_y
            && region.z + 1 < size_z && utf8(folder,512) && cells.size() == region.cells(), 5);
        std::string out = "DFMDG015";
        for (auto n : {generation,sequence,tick}) u64(out,n);
        for (auto n : {site,size_x,size_y,size_z}) u32(out,n);
        out += region.encode(); out.push_back(paused ? 1 : 0); text(out,folder);
        u32(out,static_cast<std::uint32_t>(cells.size()));
        for (const auto &c : cells) c.append(out);
        require(out.size() <= MAX_OBSERVATION,5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    bool eligible() const {
        (void)encode();
        if (!paused || sequence >= UINT64_MAX-1) return false;
        bool ok = true; std::size_t i = 0;
        region.halo([&](auto x, auto y, auto z) {
            const auto &c = cells[i++];
            // Conservative local guard, NOT a cave-in/aquifer/global-safety proof.
            if (c.presence != 2 || !c.shape || !c.material || c.dig || c.flow
                || (c.flags & 11) || c.temperature1 >= 10075 || c.temperature2 >= 10075) ok = false;
            if (region.target(x,y,z) && (c.shape != 1 || c.flags)) ok = false;
        });
        return ok;
    }
    Observation expected_after() const {
        require(eligible(),4);
        auto out = *this; ++out.sequence; std::size_t i = 0;
        region.halo([&](auto x, auto y, auto z) {
            auto &c = out.cells[i++];
            if (region.target(x,y,z)) c.dig = 1;
            if (region.touched_block(x,y,z)) c.block_designated = true;
        });
        return out;
    }
};
inline std::string plan_digest(const std::string &witness) {
    require(witness.size() == 32); return hash("dfmcp-dig-plan/1",witness);
}
inline std::string token_for(std::uint64_t generation, const std::string &key, const std::string &plan) {
    valid_key(key); require(plan.size() == 32);
    std::string out; u64(out,generation); text(out,key); out += plan;
    return hash("dfmcp-dig-token/1",out).substr(0,16);
}
enum class State : unsigned char { Prepared = 0, Unknown = 1, Designated = 2, Refused = 4 };
struct Record {
    Observation before;
    std::string key, witness, plan, token;
    Clock::time_point created;
    State state = State::Prepared;
    std::uint32_t designated_tiles = 0;
    std::string after_witness = std::string(32,'\0'), receipt = std::string(32,'\0');
    std::string prefix() const {
        std::string out = "DFMDGE15";
        for (auto n : {before.generation,before.sequence,before.tick}) u64(out,n);
        out += before.region.encode(); out += witness; out += plan; out += token;
        out.push_back(static_cast<char>(state)); u32(out,designated_tiles); out += after_witness; text(out,key);
        return out;
    }
    std::string proof() const { return hash("dfmcp-dig-receipt/1",prefix()); }
    std::string encode() const { return prefix() + receipt; }
};
class Engine {
    std::uint64_t generation_, sequence_ = 0;
    std::map<std::string,Record> records_;
public:
    explicit Engine(std::uint64_t generation, std::uint64_t sequence = 0) : generation_(generation), sequence_(sequence) {}
    std::uint64_t generation() const { return generation_; }
    std::size_t size() const { return records_.size(); }
    bool available() const { return generation_ && generation_ != UINT64_MAX && sequence_ != UINT64_MAX; }
    void interrupt() { if (sequence_ != UINT64_MAX) ++sequence_; }
    void reset() { records_.clear(); if (generation_ != UINT64_MAX) ++generation_; interrupt(); }
    bool unresolved() const {
        for (const auto &entry : records_) if (entry.second.state == State::Unknown) return true;
        return false;
    }
    template<class Capture> Observation inspect(Region region, Capture capture) const {
        require(available(),5); region.validate();
        auto out = capture(region); out.generation = generation_; out.sequence = sequence_;
        require(out.region.encode() == region.encode(),5); (void)out.encode(); return out;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        valid_key(key); require(plan.size() == 32);
        auto it = records_.find(key); if (it == records_.end()) return nullptr;
        require(it->second.plan == plan,7); return &it->second;
    }
    template<class Capture> std::pair<const Record *,bool> prepare(const std::string &key, Region region,
        const std::string &witness, const std::string &plan, Clock::time_point now, Capture capture) {
        valid_key(key); region.validate(); require(plan == plan_digest(witness),7);
        if (const auto *old = query(key,plan)) {
            require(old->before.region.encode() == region.encode(),7); return {old,true};
        }
        require(!unresolved(),8); require(records_.size() < MAX_RECORDS && available(),5);
        auto before = inspect(region,capture); require(before.eligible(),4); require(before.witness() == witness,6);
        Record record; record.before = std::move(before); record.key = key; record.witness = witness;
        record.plan = plan; record.token = token_for(generation_,key,plan); record.created = now;
        auto stored = records_.emplace(key,std::move(record)); return {&stored.first->second,false};
    }
    template<class Capture,class Apply> const Record &commit(const std::string &key, const std::string &plan,
        const std::string &token, Clock::time_point now, Capture capture, Apply apply) {
        const auto *known = query(key,plan); require(known && known->token == token && token.size() == 16,7);
        auto &record = records_.find(key)->second;
        if (record.state != State::Prepared) return record; // Unknown is NEVER retried.
        require(!unresolved(),8);
        const auto last_start = Clock::time_point::max() - LIFETIME;
        bool valid = now >= record.created && (record.created > last_start || now < record.created + LIFETIME)
            && available() && record.before.generation == generation_ && record.before.sequence == sequence_;
        try {
            if (valid) { const auto current = inspect(record.before.region,capture);
                valid = current.eligible() && current.witness() == record.witness; }
        } catch (...) { valid = false; }
        if (!valid) { record.state = State::Refused; record.receipt = record.proof(); return record; }
        // All expected-readback allocations precede the first designation write.
        const auto expected = record.before.expected_after().encode();
        interrupt(); record.state = State::Unknown;
        try {
            apply(record.before.region);
            const auto after = inspect(record.before.region,capture).encode();
            if (after != expected) return record;
            auto terminal = record; terminal.state = State::Designated;
            terminal.designated_tiles = record.before.region.width * record.before.region.height;
            terminal.after_witness = dfmcp_snapshot::sha256(after); terminal.receipt = terminal.proof();
            record = std::move(terminal);
        } catch (...) { /* A subset may have changed. Never undo or blindly retry. */ }
        return record;
    }
};
}
