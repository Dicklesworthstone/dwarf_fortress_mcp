#pragma once
#include "retained_snapshot.h"
#include <cstdint>
#include <exception>
#include <map>
#include <string>
#include <tuple>
#include <vector>

// A bounded normal-mining designation, NOT excavation or an atomic game batch.
// All callbacks execute within one DFHack-owned suspension. No native pointer
// survives a callback. Durable dispatch ownership belongs to an external caller.
namespace dfmcp_dig_v1_16 {
using Clock = std::chrono::steady_clock;
constexpr std::size_t MAX_RECORDS = 128, MAX_CELLS = 300, MAX_BYTES = 16 * 1024;
constexpr std::chrono::seconds PREPARE_LIFETIME{60};
constexpr std::uint32_t PRIORITY = 4000;
constexpr std::uint64_t MAX_TICK = std::uint64_t{UINT32_MAX} * 403200 + 403199;
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t value) : code(value) {}
    const char *what() const noexcept override { return "dig designation refused"; }
};
inline void require(bool valid, std::uint32_t code = 3) { if (!valid) throw Failure(code); }
inline void u16(std::string &out, std::uint16_t n) {
    out.push_back(static_cast<char>(n >> 8)); out.push_back(static_cast<char>(n));
}
inline void u32(std::string &out, std::uint32_t n) {
    for (int shift = 24; shift >= 0; shift -= 8) out.push_back(static_cast<char>(n >> shift));
}
inline void u64(std::string &out, std::uint64_t n) {
    for (int shift = 56; shift >= 0; shift -= 8) out.push_back(static_cast<char>(n >> shift));
}
inline bool utf8(const std::string &value, std::size_t maximum) {
    if (value.empty() || value.size() > maximum) return false;
    for (std::size_t i = 0; i < value.size();) {
        const auto c = static_cast<unsigned char>(value[i]);
        if (!c) return false;
        if (c < 128) { ++i; continue; }
        const std::size_t n = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
        if (!n || value.size() - i < n) return false;
        for (std::size_t k = 1; k < n; ++k)
            if ((static_cast<unsigned char>(value[i+k]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(value[i+1]);
        if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0)
            || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void text(std::string &out, const std::string &value) {
    require(value.size() <= UINT16_MAX); u16(out, static_cast<std::uint16_t>(value.size())); out += value;
}
inline void valid_key(const std::string &key) {
    require(!key.empty() && key.size() <= 128);
    for (unsigned char c : key)
        require((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
            || (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.');
}
inline std::string domain(const char *value) { std::string out(value); out.push_back('\0'); return out; }

struct Region {
    std::uint32_t x = 0, y = 0, z = 0, width = 0, height = 0;
    void validate() const {
        require(width >= 1 && width <= 8 && height >= 1 && height <= 8
            && x >= 1 && y >= 1 && z >= 1 && z <= 32766
            && x <= 32767 - width && y <= 32767 - height);
    }
    bool operator==(const Region &b) const {
        return std::tie(x,y,z,width,height) == std::tie(b.x,b.y,b.z,b.width,b.height);
    }
    bool target(std::uint32_t px, std::uint32_t py, std::uint32_t pz) const {
        return pz == z && px >= x && px - x < width && py >= y && py - y < height;
    }
    bool target_block(std::uint32_t px, std::uint32_t py, std::uint32_t pz) const {
        return pz == z && (px >> 4) >= (x >> 4) && (px >> 4) <= ((x+width-1) >> 4)
            && (py >> 4) >= (y >> 4) && (py >> 4) <= ((y+height-1) >> 4);
    }
    std::size_t cells() const { validate(); return (width + 2) * (height + 2) * 3; }
    void append(std::string &out) const { validate(); for (auto n : {x,y,z,width,height}) u32(out,n); }
};

// Hidden/missing cells contain only their presence tag on the wire. Hazards are
// observed only on visible cells: 1=liquid, 2=aquifer, 4=feature, 8=temperature.
struct Cell {
    std::uint8_t presence = 0, dig = 0, hazards = 0;
    bool natural_wall = false, smooth = false, occupied = false, job = false, designated = false;
    std::uint32_t tiletype = 0, designation_other = 0, occupancy = 0, priority = 0, cooldown = 0, block_other = 0;
    std::uint16_t temperature1 = 0, temperature2 = 0;
    void append(std::string &out) const {
        require(presence <= 2 && dig <= 7 && hazards <= 15 && priority <= 7000, 5);
        out.push_back(static_cast<char>(presence));
        if (presence != 2) {
            require(!dig && !hazards && !natural_wall && !smooth && !occupied && !job && !designated
                && !tiletype && !designation_other && !occupancy && !priority && !cooldown && !block_other
                && !temperature1 && !temperature2, 5);
            return;
        }
        for (auto n : {tiletype,designation_other,occupancy,priority,cooldown,block_other}) u32(out,n);
        u16(out,temperature1); u16(out,temperature2);
        out.push_back(static_cast<char>(dig)); out.push_back(static_cast<char>(hazards));
        out.push_back(static_cast<char>((natural_wall ? 1 : 0) | (smooth ? 2 : 0)
            | (occupied ? 4 : 0) | (job ? 8 : 0) | (designated ? 16 : 0)));
    }
};
enum Blocker : std::uint32_t {
    Unpaused = 1, UnobservedTarget = 2, NotNaturalWall = 4, ExistingDesignation = 8,
    OccupiedOrJob = 16, KnownHazard = 32, MissingContext = 64, HiddenContext = 128,
};
struct Observation {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0, size_x = 0, size_y = 0, size_z = 0;
    bool paused = false;
    std::string folder;
    Region region;
    std::vector<Cell> tiles;
    template<class Visit> void each(Visit visit) const {
        region.validate(); require(tiles.size() == region.cells(), 5);
        std::size_t i = 0;
        for (auto z = region.z-1; z <= region.z+1; ++z)
        for (auto y = region.y-1; y <= region.y+region.height; ++y)
        for (auto x = region.x-1; x <= region.x+region.width; ++x) visit(i++,x,y,z);
    }
    std::string encode() const {
        region.validate();
        require(generation && generation < UINT64_MAX && sequence < UINT64_MAX && tick <= MAX_TICK
            && site <= INT32_MAX && utf8(folder,512), 5);
        require(size_x > region.x+region.width && size_x <= 32768
            && size_y > region.y+region.height && size_y <= 32768
            && size_z > region.z+1 && size_z <= 32768, 5);
        require(tiles.size() == region.cells() && tiles.size() <= MAX_CELLS, 5);
        std::string out = "DFMDG016";
        u64(out,generation); u64(out,sequence); u64(out,tick);
        for (auto n : {site,size_x,size_y,size_z}) u32(out,n);
        region.append(out); out.push_back(paused ? 1 : 0); text(out,folder);
        u16(out,static_cast<std::uint16_t>(tiles.size()));
        for (const auto &cell : tiles) cell.append(out);
        require(out.size() <= MAX_BYTES,5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    std::uint32_t blockers(bool allow_hidden_neighbors) const {
        (void)encode(); std::uint32_t out = paused ? 0u : static_cast<std::uint32_t>(Unpaused);
        each([&](std::size_t i, auto x, auto y, auto z) {
            const auto &c = tiles[i];
            if (c.presence == 0) out |= MissingContext;
            if (c.presence == 1 && !allow_hidden_neighbors) out |= HiddenContext;
            if (c.presence == 2 && c.hazards) out |= KnownHazard;
            if (!region.target(x,y,z)) return;
            if (c.presence != 2) out |= UnobservedTarget;
            else {
                if (!c.natural_wall) out |= NotNaturalWall;
                if (c.dig || c.smooth) out |= ExistingDesignation;
                if (c.occupied || c.job) out |= OccupiedOrJob;
            }
        });
        return out;
    }
    Observation expected_after() const {
        require(sequence < UINT64_MAX-1,5);
        auto out = *this; ++out.sequence;
        each([&](std::size_t i, auto x, auto y, auto z) {
            auto &cell = out.tiles[i];
            if (cell.presence != 2) return;
            if (region.target(x,y,z)) { cell.dig = 1; cell.priority = PRIORITY; }
            if (region.target_block(x,y,z)) { cell.designated = true; cell.cooldown = 0; }
        });
        return out;
    }
};
inline std::string plan_digest(const Region &region, bool allow_hidden, const std::string &witness) {
    require(witness.size() == 32); auto bytes = domain("dfmcp-dig-designation-plan/1");
    region.append(bytes); bytes.push_back(allow_hidden ? 1 : 0); bytes += witness;
    return dfmcp_snapshot::sha256(bytes);
}
inline std::string prepare_token(std::uint64_t generation, const std::string &key, const std::string &plan) {
    valid_key(key); require(plan.size() == 32);
    auto bytes = domain("dfmcp-dig-designation-token/1"); u64(bytes,generation); text(bytes,key); bytes += plan;
    return dfmcp_snapshot::sha256(bytes).substr(0,16);
}
enum class State : std::uint8_t { Prepared = 0, Unknown = 1, Designated = 2, Refused = 4 };
enum class Reason : std::uint8_t { None = 0, Stale = 1, Cancelled = 2 };
struct Record {
    std::string key, plan, witness, token;
    Observation before;
    bool allow_hidden = false;
    State state = State::Prepared;
    Reason reason = Reason::None;
    Clock::time_point created;
    bool after_known = false;
    std::uint32_t designated_count = 0;
    std::string after_witness = std::string(32,'\0'), receipt = std::string(32,'\0');
    bool terminal() const { return state == State::Designated || state == State::Refused; }
    std::string proof() const {
        auto bytes = domain("dfmcp-dig-designation-receipt/1");
        u64(bytes,before.generation); text(bytes,key); bytes += plan; bytes += token;
        bytes.push_back(static_cast<char>(state)); bytes.push_back(static_cast<char>(reason));
        bytes.push_back(after_known ? 1 : 0); u32(bytes,designated_count); bytes += after_witness;
        return dfmcp_snapshot::sha256(bytes);
    }
    std::string encode() const {
        std::string bytes = "DFMDGE16"; u64(bytes,before.generation); u64(bytes,before.sequence); u64(bytes,before.tick);
        before.region.append(bytes); bytes.push_back(allow_hidden ? 1 : 0);
        bytes += witness; bytes += plan; bytes += token;
        bytes.push_back(static_cast<char>(state)); bytes.push_back(static_cast<char>(reason));
        bytes.push_back(after_known ? 1 : 0); u32(bytes,designated_count); bytes += after_witness; bytes += receipt; text(bytes,key);
        return bytes;
    }
};
class Engine {
    std::uint64_t generation_, sequence_ = 0;
    std::map<std::string,Record> records_;
    static void retire(Record &r, Reason why) {
        // Retire before allocating a receipt. A failed allocation never revives dispatch.
        r.state = State::Refused; r.reason = why; r.receipt = r.proof();
    }
public:
    explicit Engine(std::uint64_t generation) : generation_(generation) {}
    std::uint64_t generation() const { return generation_; }
    std::uint64_t sequence() const { return sequence_; }
    std::size_t size() const { return records_.size(); }
    bool available() const { return generation_ && generation_ < UINT64_MAX && sequence_ < UINT64_MAX; }
    void interrupt() { if (sequence_ < UINT64_MAX) ++sequence_; }
    void reset() { records_.clear(); if (generation_ < UINT64_MAX) ++generation_; interrupt(); }
    bool unresolved() const {
        for (const auto &entry : records_) if (entry.second.state == State::Unknown) return true;
        return false;
    }
    template<class Read> Observation inspect(const Region &region, Read read) const {
        region.validate(); require(available(),5); auto value = read(region);
        require(value.region == region,5); value.generation = generation_; value.sequence = sequence_;
        (void)value.encode(); return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        valid_key(key); require(plan.size() == 32);
        const auto it = records_.find(key); if (it == records_.end()) return nullptr;
        require(it->second.plan == plan,7); return &it->second;
    }
    template<class Read> std::pair<const Record *,bool> prepare(const std::string &key, const Region &region,
        bool allow_hidden, const std::string &witness, const std::string &plan, Clock::time_point now, Read read) {
        valid_key(key); require(plan == plan_digest(region,allow_hidden,witness),7);
        if (const auto *old = query(key,plan)) return {old,true};
        require(!unresolved(),8); require(records_.size() < MAX_RECORDS,3);
        auto before = inspect(region,read);
        require(before.witness() == witness,6); require(before.blockers(allow_hidden) == 0 && before.sequence < UINT64_MAX-1,4);
        Record r; r.key = key; r.plan = plan; r.witness = witness; r.allow_hidden = allow_hidden;
        r.token = prepare_token(generation_,key,plan); r.before = std::move(before); r.created = now;
        const auto stored = records_.emplace(key,std::move(r)); return {&stored.first->second,false};
    }
    const Record &cancel(const std::string &key, const std::string &plan, const std::string &token) {
        const auto *known = query(key,plan); require(known && token.size() == 16 && token == known->token,7);
        auto &r = records_.find(key)->second;
        if (r.state == State::Prepared) retire(r,Reason::Cancelled);
        // Unknown/Designated cannot be undone or reclassified by cancellation.
        return r;
    }
    template<class Read, class Write> const Record &commit(const std::string &key, const std::string &plan,
        const std::string &token, Clock::time_point now, Read read, Write write) {
        const auto *known = query(key,plan); require(known && token.size() == 16 && token == known->token,7);
        auto &r = records_.find(key)->second;
        if (r.state != State::Prepared) return r;
        require(!unresolved(),8);
        const auto latest = Clock::time_point::max() - PREPARE_LIFETIME;
        bool valid = now >= r.created && (r.created > latest || now < r.created + PREPARE_LIFETIME)
            && available() && r.before.generation == generation_ && r.before.sequence == sequence_;
        try { if (valid) { auto current = inspect(r.before.region,read);
            valid = current.witness() == r.witness && current.blockers(r.allow_hidden) == 0; }
        } catch (...) { valid = false; }
        if (!valid) { retire(r,Reason::Stale); return r; }
        // Construct the precise allowed post-state before any write. This includes
        // priority and block scheduling, not just the requested dig bits.
        const auto expected = r.before.expected_after().witness();
        interrupt(); r.state = State::Unknown; // Publish uncertainty BEFORE all native writes.
        try {
            write(r.before.region);
            const auto after = inspect(r.before.region,read);
            if (after.witness() != expected) return r;
            auto candidate = r; candidate.state = State::Designated; candidate.after_known = true;
            candidate.designated_count = r.before.region.width * r.before.region.height;
            candidate.after_witness = expected; candidate.receipt = candidate.proof();
            r = std::move(candidate);
        } catch (...) { /* Partial writes are possible. No rollback and no retry. */ }
        return r;
    }
};
} // namespace dfmcp_dig_v1_16
