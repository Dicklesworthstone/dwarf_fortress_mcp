#pragma once
#include "retained_snapshot.h"
#include <exception>
#include <optional>
#include <vector>

// furniture/1.19: one ordinary bed, chair or table, using one exact existing
// item. Callbacks run in the same suspended native dispatch. This value engine
// grants no authority and retains no native pointers. Bead: df-dfhack-bridge-plane-c-pic.4.
namespace dfmcp_build {
constexpr std::size_t MAX_RECORDS = 256, MAX_BUILDINGS = 65536;
constexpr std::size_t MAX_CAPTURE_BYTES = 2048, MAX_RECORD_BYTES = 6144;
constexpr std::uint64_t PREPARE_MS = 60000;
constexpr std::uint64_t MAX_TICK = std::uint64_t{UINT32_MAX} * 403200 + 403199;
// Pinned DF item_flags: these cache bookkeeping bits do not claim, move, hide,
// forbid or consume an item. Preserve their exact bytes in every witness.
constexpr std::uint32_t COMPUTED_ITEM_FLAGS = (std::uint32_t{1} << 28) | (std::uint32_t{1} << 29);
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t c) : code(c) {}
    const char *what() const noexcept override { return "furniture placement refused"; }
};
inline void require(bool ok, std::uint32_t code = 3) { if (!ok) throw Failure(code); }
inline void u32(std::string &out, std::uint32_t n) {
    for (int s = 24; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void u64(std::string &out, std::uint64_t n) {
    for (int s = 56; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void field(std::string &out, const std::string &v) {
    require(v.size() <= UINT16_MAX);
    out.push_back(static_cast<char>(v.size() >> 8));
    out.push_back(static_cast<char>(v.size())); out += v;
}
inline bool utf8(const std::string &s, std::size_t max) {
    if (s.empty() || s.size() > max) return false;
    for (std::size_t i = 0; i < s.size();) {
        const auto a = static_cast<unsigned char>(s[i]);
        if (!a) return false;
        if (a < 128) { ++i; continue; }
        const std::size_t n = a >= 0xc2 && a <= 0xdf ? 2 : a >= 0xe0 && a <= 0xef ? 3 : a >= 0xf0 && a <= 0xf4 ? 4 : 0;
        if (!n || s.size() - i < n) return false;
        for (std::size_t j = 1; j < n; ++j)
            if ((static_cast<unsigned char>(s[i+j]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(s[i+1]);
        if ((a == 0xe0 && b < 0xa0) || (a == 0xed && b >= 0xa0)
            || (a == 0xf0 && b < 0x90) || (a == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void key_valid(const std::string &key) {
    require(!key.empty() && key.size() <= 128);
    for (unsigned char c : key)
        require((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
            || (c >= '0' && c <= '9') || c == '.' || c == '-' || c == '_');
}
inline std::string hash(const char *domain, const std::string &data) {
    std::string out(domain); out.push_back('\0'); out += data;
    return dfmcp_snapshot::sha256(out);
}
inline void optional_id(std::string &out, const std::optional<std::uint32_t> &id) {
    out.push_back(id ? 1 : 0);
    if (id) { require(*id < INT32_MAX, 5); u32(out, *id); }
}
enum class Kind : unsigned char { Other = 0, Bed = 1, Chair = 2, Table = 3 };
enum class Presence : unsigned char { Missing = 0, Hidden = 1, Visible = 2 };
struct Coord {
    std::uint32_t x = 0, y = 0, z = 0;
    bool operator==(const Coord &v) const { return x == v.x && y == v.y && z == v.z; }
    std::string encode() const {
        require(x < 32768 && y < 32768 && z < 32768);
        std::string out; u32(out, x); u32(out, y); u32(out, z); return out;
    }
};
struct Selection {
    Kind kind = Kind::Bed;
    std::uint32_t item = 0;
    Coord target;
    std::string encode() const {
        require(static_cast<unsigned>(kind) >= 1 && static_cast<unsigned>(kind) <= 3 && item < INT32_MAX);
        // A complete 3x3 same-level context is mandatory, including at map edges.
        require(target.x > 0 && target.y > 0 && target.x < 32767 && target.y < 32767);
        std::string out(1, static_cast<char>(kind)); u32(out, item); out += target.encode(); return out;
    }
};
struct Tile {
    Presence presence = Presence::Missing;
    std::uint32_t tiletype = 0, occupancy_other = 0;
    unsigned char shape = 0, liquid = 0, dig = 0;
    bool occupied = false;
    std::optional<std::uint32_t> building;
    std::string encode() const {
        require(static_cast<unsigned>(presence) <= 2, 5);
        std::string out(1, static_cast<char>(presence));
        if (presence != Presence::Visible) {
            require(!tiletype && !occupancy_other && !shape && !liquid && !dig && !occupied && !building, 5);
            return out;
        }
        require(tiletype <= INT32_MAX && shape <= 8 && liquid <= 7 && dig <= 7, 5);
        u32(out, tiletype); out.push_back(static_cast<char>(shape));
        out.push_back(static_cast<char>(liquid)); out.push_back(static_cast<char>(dig));
        u32(out, occupancy_other); out.push_back(occupied ? 1 : 0); optional_id(out, building); return out;
    }
    bool dry() const { return presence == Presence::Visible && liquid == 0; }
    bool empty_floor() const { return dry() && shape == 3 && dig == 0 && !occupied && !building && !occupancy_other; }
};
struct Item {
    Presence presence = Presence::Missing;
    Coord pos;
    Kind kind = Kind::Other;
    std::uint32_t native_type = 0, quality = 0, wear = 0, other_flags = 0, other_refs = 0;
    std::int32_t subtype = 0, material = 0, material_index = 0;
    bool on_ground = false, in_job = false;
    std::vector<std::uint32_t> jobs;
    Tile ground;
    std::string encode() const {
        require(static_cast<unsigned>(presence) <= 2, 5);
        std::string out(1, static_cast<char>(presence));
        if (presence != Presence::Visible) {
            require(pos == Coord{} && kind == Kind::Other && !native_type && !quality && !wear
                && !other_flags && !other_refs && !subtype && !material && !material_index
                && !on_ground && !in_job && jobs.empty() && ground.encode() == std::string(1, '\0'), 5);
            return out;
        }
        require(static_cast<unsigned>(kind) <= 3 && native_type <= INT32_MAX
            && quality <= INT32_MAX && wear <= INT32_MAX && other_refs <= 4096 && jobs.size() <= 8, 5);
        out += pos.encode(); out.push_back(static_cast<char>(kind)); u32(out, native_type);
        for (auto n : {subtype, material, material_index}) u32(out, static_cast<std::uint32_t>(n));
        u32(out, quality); u32(out, wear); u32(out, other_flags); u32(out, other_refs);
        out.push_back(on_ground ? 1 : 0); out.push_back(in_job ? 1 : 0);
        out.push_back(static_cast<char>(jobs.size()));
        for (std::size_t i = 0; i < jobs.size(); ++i) {
            require(jobs[i] < INT32_MAX && (!i || jobs[i-1] < jobs[i]), 5); u32(out, jobs[i]);
        }
        require(ground.presence == Presence::Visible, 5); out += ground.encode(); return out;
    }
    bool available(Kind expected) const {
        return presence == Presence::Visible && kind == expected && on_ground && !in_job
            && !(other_flags & ~COMPUTED_ITEM_FLAGS) && !other_refs && jobs.empty() && wear == 0 && material >= 0
            && ground.dry() && ground.shape == 3 && !ground.occupied;
    }
};
struct Capture {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0, next_building = 0, next_job = 0, building_count = 0;
    std::array<std::uint32_t, 3> dimensions{};
    std::string folder;
    bool paused = false, free_tile = false, supported = false;
    Selection selection;
    std::array<Tile, 9> tiles{}; // y-major 3x3; index 4 is the target.
    Item item;
    std::string encode() const {
        require(generation && generation < UINT64_MAX && sequence < UINT64_MAX && tick <= MAX_TICK, 5);
        require(site <= INT32_MAX && next_building <= INT32_MAX && next_job <= INT32_MAX
            && building_count <= MAX_BUILDINGS && utf8(folder, 512), 5);
        for (auto d : dimensions) require(d > 0 && d <= 32768, 5);
        const auto &p = selection.target;
        require(p.x + 1 < dimensions[0] && p.y + 1 < dimensions[1] && p.z < dimensions[2], 5);
        std::string out = "DFMBC019"; u64(out, generation); u64(out, sequence); u64(out, tick);
        u32(out, site); for (auto d : dimensions) u32(out, d);
        u32(out, next_building); u32(out, next_job); u32(out, building_count); field(out, folder);
        out.push_back(paused ? 1 : 0); out.push_back(free_tile ? 1 : 0); out.push_back(supported ? 1 : 0);
        out += selection.encode(); for (const auto &t : tiles) out += t.encode();
        if (item.presence == Presence::Visible)
            require(item.pos.x < dimensions[0] && item.pos.y < dimensions[1] && item.pos.z < dimensions[2], 5);
        out += item.encode(); require(out.size() <= MAX_CAPTURE_BYTES, 5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    bool eligible() const {
        (void)encode();
        if (!paused || !free_tile || !supported || sequence >= UINT64_MAX - 1
            || next_building >= INT32_MAX || next_job >= INT32_MAX || building_count >= MAX_BUILDINGS
            || !tiles[4].empty_floor() || !item.available(selection.kind)) return false;
        for (const auto &t : tiles) if (!t.dry()) return false;
        // A local accessible-looking neighbor, not a pathfinding/safety certificate.
        return tiles[1].empty_floor() || tiles[3].empty_floor() || tiles[5].empty_floor() || tiles[7].empty_floor();
    }
    Capture expected_after() const {
        require(eligible(), 4);
        auto out = *this; ++out.sequence; ++out.next_building; ++out.next_job; ++out.building_count;
        out.free_tile = false; out.tiles[4].occupied = true; out.tiles[4].building = next_building;
        out.item.in_job = true; out.item.jobs = {next_job}; return out;
    }
};
inline std::string plan_digest(const Selection &s, const std::string &witness) {
    require(witness.size() == 32); return hash("dfmcp-build-plan/1", s.encode() + witness);
}
inline std::string token_for(const std::string &key, const std::string &plan) {
    key_valid(key); require(plan.size() == 32);
    std::string data; field(data, key); data += plan; return hash("dfmcp-build-token/1", data).substr(0, 16);
}
// The native verifier must inspect the registry, building, linked ConstructBuilding
// job, job holder, exact Hauled item, reverse item->job link and fixed fields.
// stage/max_stage distinguish job registration from a completed building.
struct Insertion {
    std::uint32_t building = 0, job = 0, item = 0, stage = 0, max_stage = 0;
    Kind kind = Kind::Other;
    Coord pos;
    std::int32_t material = 0, material_index = 0;
    bool linked = false, construct_job = false, exact_item_link = false, suspended = false;
    std::string encode() const {
        require(building < INT32_MAX && job < INT32_MAX && item < INT32_MAX
            && static_cast<unsigned>(kind) >= 1 && static_cast<unsigned>(kind) <= 3
            && max_stage >= 1 && max_stage <= 32 && stage <= max_stage, 5);
        std::string out = "DFMBI019"; u32(out, building); u32(out, job); u32(out, item);
        out.push_back(static_cast<char>(kind)); out += pos.encode();
        u32(out, static_cast<std::uint32_t>(material)); u32(out, static_cast<std::uint32_t>(material_index));
        u32(out, stage); u32(out, max_stage);
        for (bool b : {linked, construct_job, exact_item_link, suspended}) out.push_back(b ? 1 : 0);
        return out;
    }
    bool matches(const Capture &c) const {
        (void)encode();
        return building == c.next_building && job == c.next_job && item == c.selection.item
            && kind == c.selection.kind && pos == c.selection.target && material == c.item.material
            && material_index == c.item.material_index && stage == 0
            && linked && construct_job && exact_item_link && !suspended;
    }
};
enum class Phase : unsigned char { Prepared = 0, Indeterminate = 1, Placed = 2, Refused = 3, Cancelled = 4 };
enum class Reason : unsigned char { None = 0, Stale = 1, Expired = 2, SourceChanged = 3, Cancelled = 4, NativeFailure = 5 };
struct Record {
    std::string key, plan, token;
    Capture before;
    std::uint64_t created_ms = 0; // local monotonic lifetime only, not serialized authority.
    Phase phase = Phase::Prepared;
    Reason reason = Reason::None;
    std::optional<Capture> after;
    std::optional<Insertion> insertion;
    bool attempted() const { return phase == Phase::Indeterminate || phase == Phase::Placed; }
    bool terminal() const { return phase == Phase::Placed || phase == Phase::Refused || phase == Phase::Cancelled; }
    std::string encode() const {
        key_valid(key); require(plan == plan_digest(before.selection, before.witness()) && token == token_for(key, plan), 5);
        require(before.eligible(), 5);
        require((phase == Phase::Prepared && reason == Reason::None)
            || (phase == Phase::Indeterminate && reason == Reason::NativeFailure)
            || (phase == Phase::Placed && reason == Reason::None)
            || (phase == Phase::Refused && (reason == Reason::Stale || reason == Reason::Expired || reason == Reason::SourceChanged))
            || (phase == Phase::Cancelled && reason == Reason::Cancelled), 5);
        require(after.has_value() == (phase == Phase::Placed) && insertion.has_value() == after.has_value(), 5);
        std::string out = "DFMBR019"; field(out, key); field(out, before.encode()); out += plan; out += token;
        out.push_back(static_cast<char>(phase)); out.push_back(static_cast<char>(reason)); out.push_back(attempted() ? 1 : 0);
        out.push_back(after ? 1 : 0);
        if (after) {
            require(after->encode() == before.expected_after().encode() && insertion->matches(before), 5);
            field(out, after->encode()); field(out, insertion->encode());
        }
        out += hash("dfmcp-build-receipt/1", out); require(out.size() <= MAX_RECORD_BYTES, 5); return out;
    }
};
class Engine {
    struct Busy {
        bool &flag;
        explicit Busy(bool &v) : flag(v) { require(!flag, 8); flag = true; }
        ~Busy() { flag = false; }
        Busy(const Busy &) = delete;
        Busy &operator=(const Busy &) = delete;
    };
public:
    explicit Engine(std::uint64_t generation) : generation_(generation) {}
    std::uint64_t generation() const { return generation_; }
    std::uint64_t sequence() const { return sequence_; }
    bool available() const { return generation_ && generation_ < UINT64_MAX && sequence_ < UINT64_MAX; }
    std::size_t size() const { return records_.size(); }
    bool unresolved() const { return unresolved_; }
    void interrupt() { if (sequence_ < UINT64_MAX) ++sequence_; }
    void change_source() { if (generation_ < UINT64_MAX) ++generation_; interrupt(); }
    template<class Reader> Capture inspect(const Selection &selection, Reader read) const {
        require(available(), 5); const auto g = generation_, s = sequence_;
        (void)selection.encode(); auto value = read(selection);
        require(g == generation_ && s == sequence_, 6);
        value.generation = g; value.sequence = s;
        require(value.selection.encode() == selection.encode(), 6); (void)value.encode(); return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        key_valid(key); require(plan.size() == 32);
        const auto it = records_.find(key); if (it == records_.end()) return nullptr;
        require(it->second.plan == plan, 7); return &it->second;
    }
    template<class Reader> const Record &prepare(const std::string &key, const Selection &selection,
        const std::string &witness, const std::string &plan, std::uint64_t now_ms, Reader read) {
        Busy owned(busy_); key_valid(key); require(plan == plan_digest(selection, witness), 7);
        if (const auto *old = query(key, plan)) return *old; // Replay cannot renew preparation.
        require(!unresolved_, 8); require(records_.size() < MAX_RECORDS, 5);
        auto before = inspect(selection, read); require(before.eligible(), 4); require(before.witness() == witness, 6);
        Record r; r.key = key; r.plan = plan; r.token = token_for(key, plan); r.before = std::move(before); r.created_ms = now_ms;
        (void)r.encode(); return records_.emplace(key, std::move(r)).first->second;
    }
    const Record &cancel(const std::string &key, const std::string &plan, const std::string &token) {
        Busy owned(busy_); auto &r = authenticated(key, plan, token);
        if (r.phase == Phase::Prepared) { r.phase = Phase::Cancelled; r.reason = Reason::Cancelled; }
        // Never deconstruct or detach a job/item after any placement attempt.
        return r;
    }
    template<class Reader, class Writer, class Verifier> const Record &commit(const std::string &key,
        const std::string &plan, const std::string &token, std::uint64_t now_ms,
        Reader read, Writer write, Verifier verify) {
        Busy owned(busy_); auto &r = authenticated(key, plan, token);
        if (r.phase != Phase::Prepared) return r;
        require(!unresolved_, 8);
        Reason refusal = Reason::None;
        if (r.before.generation != generation_) refusal = Reason::SourceChanged;
        else if (now_ms < r.created_ms || now_ms - r.created_ms >= PREPARE_MS) refusal = Reason::Expired;
        else {
            try {
                const auto current = inspect(r.before.selection, read);
                if (!current.eligible() || current.encode() != r.before.encode()) refusal = Reason::Stale;
            } catch (...) { refusal = Reason::Stale; }
        }
        if (refusal != Reason::None) { r.phase = Phase::Refused; r.reason = refusal; return r; }
        // Complete all expected-state allocations before crossing the effect boundary.
        const auto expected = r.before.expected_after().encode();
        interrupt(); r.phase = Phase::Indeterminate; r.reason = Reason::NativeFailure; unresolved_ = true;
        try { write(r.before); }
        catch (...) { /* DFHack can throw after linking a building or job. Readback decides. */ }
        try {
            const auto after = inspect(r.before.selection, read);
            if (after.encode() != expected) return r;
            const auto proof = verify(r.before);
            if (!proof.matches(r.before) || inspect(r.before.selection, read).encode() != expected) return r;
            Record next = r; next.after = after; next.insertion = proof;
            next.phase = Phase::Placed; next.reason = Reason::None;
            (void)next.encode(); r = std::move(next); unresolved_ = false;
        } catch (...) { /* No rollback, no retry, no invented nonapplication. */ }
        return r;
    }
private:
    std::uint64_t generation_, sequence_ = 0;
    bool unresolved_ = false, busy_ = false;
    std::map<std::string, Record> records_;
    Record &authenticated(const std::string &key, const std::string &plan, const std::string &token) {
        const auto *r = query(key, plan); require(r && token.size() == 16 && r->token == token, 7);
        return records_.find(key)->second;
    }
};
} // namespace dfmcp_build
