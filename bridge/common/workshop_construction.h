#pragma once
#include "retained_snapshot.h"
#include <array>
#include <exception>
#include <map>
#include <optional>
#include <vector>

// workshop/1.20: one Carpenter's workshop with a fixed 3x3 footprint, built
// from one exact existing ground WOOD log. This is a separate generation from
// furniture/1.19: it shares no capture, plan, token, receipt or writer bytes.
// Callbacks run inside one suspended native dispatch. The value engine grants
// no authority and retains no native pointers. Bead:
// dfmcp-reality-wiring-proof-4bf.14.
namespace dfmcp_workshop {
constexpr std::size_t MAX_RECORDS = 64, MAX_BUILDINGS = 65536, MAX_WORKERS = 32;
// Worst case: capture 1,629 bytes, record 3,563 bytes; every reply fits 8 KiB.
constexpr std::size_t MAX_CAPTURE_BYTES = 2048, MAX_RECORD_BYTES = 6144, MAX_PROGRESS_BYTES = 1024;
constexpr std::uint64_t PREPARE_MS = 60000;
constexpr std::uint64_t MAX_TICK = std::uint64_t{UINT32_MAX} * 403200 + 403199;
// Pinned DF item_flags bookkeeping bits (temps_computed, weight_computed). They
// never claim, move, hide, forbid or consume an item; their bytes stay witnessed.
constexpr std::uint32_t COMPUTED_ITEM_FLAGS = (std::uint32_t{1} << 28) | (std::uint32_t{1} << 29);
constexpr std::uint16_t ALL_TILES = 0x1ff;
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t c) : code(c) {}
    const char *what() const noexcept override { return "workshop construction refused"; }
};
inline void require(bool ok, std::uint32_t code = 3) { if (!ok) throw Failure(code); }
inline void u16(std::string &out, std::uint16_t n) {
    out.push_back(static_cast<char>(n >> 8)); out.push_back(static_cast<char>(n));
}
inline void u32(std::string &out, std::uint32_t n) {
    for (int s = 24; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void i32(std::string &out, std::int32_t n) { u32(out, static_cast<std::uint32_t>(n)); }
inline void u64(std::string &out, std::uint64_t n) {
    for (int s = 56; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void flag(std::string &out, bool b) { out.push_back(b ? 1 : 0); }
inline void field(std::string &out, const std::string &v) {
    require(v.size() <= UINT16_MAX); u16(out, static_cast<std::uint16_t>(v.size())); out += v;
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
    flag(out, id.has_value());
    if (id) { require(*id < INT32_MAX, 5); u32(out, *id); }
}
// The closed profile has exactly one workshop kind.
enum class Kind : unsigned char { Carpenters = 1 };
enum class Presence : unsigned char { Missing = 0, Hidden = 1, Visible = 2 };
struct Coord {
    std::uint32_t x = 0, y = 0, z = 0;
    bool operator==(const Coord &v) const { return x == v.x && y == v.y && z == v.z; }
    std::string encode() const {
        require(x < 32768 && y < 32768 && z < 32768);
        std::string out; u32(out, x); u32(out, y); u32(out, z); return out;
    }
};
// One exact log and the fixed 3x3 footprint's center tile.
struct Selection {
    Kind kind = Kind::Carpenters;
    std::uint32_t item = 0;
    Coord center;
    std::string encode() const {
        require(kind == Kind::Carpenters && item < INT32_MAX);
        require(center.x > 0 && center.y > 0 && center.x < 32767 && center.y < 32767);
        std::string out(1, static_cast<char>(kind)); u32(out, item); out += center.encode(); return out;
    }
    Coord corner() const { return {center.x - 1, center.y - 1, center.z}; }
};
struct Tile {
    Presence presence = Presence::Missing;
    std::uint32_t tiletype = 0, occupancy_other = 0;
    unsigned char shape = 0, liquid = 0, dig = 0, smooth = 0;
    bool pile = false, occupied = false, zone = false;
    std::optional<std::uint32_t> building;
    std::string encode() const {
        require(static_cast<unsigned>(presence) <= 2, 5);
        std::string out(1, static_cast<char>(presence));
        if (presence != Presence::Visible) {
            require(!tiletype && !occupancy_other && !shape && !liquid && !dig && !smooth
                && !pile && !occupied && !zone && !building, 5);
            return out;
        }
        require(tiletype <= INT32_MAX && shape <= 8 && liquid <= 7 && dig <= 7 && smooth <= 3, 5);
        u32(out, tiletype); out.push_back(static_cast<char>(shape));
        out.push_back(static_cast<char>(liquid)); out.push_back(static_cast<char>(dig));
        flag(out, pile); out.push_back(static_cast<char>(smooth));
        u32(out, occupancy_other); flag(out, occupied); optional_id(out, building); flag(out, zone);
        return out;
    }
    bool dry() const { return presence == Presence::Visible && liquid == 0; }
    // Units and items count as occupancy: the footprint must be entirely clear.
    bool empty_floor() const {
        return dry() && shape == 3 && dig == 0 && !pile && !smooth && !occupied && !building
            && !occupancy_other && !zone;
    }
};
struct Item {
    Presence presence = Presence::Missing;
    Coord pos;
    bool log = false, wood = false, on_ground = false, in_job = false, reachable = false;
    std::uint32_t native_type = 0, quality = 0, wear = 0, other_flags = 0, other_refs = 0;
    std::int32_t subtype = 0, material = 0, material_index = 0;
    std::vector<std::uint32_t> jobs;
    Tile ground;
    std::string encode() const {
        require(static_cast<unsigned>(presence) <= 2, 5);
        std::string out(1, static_cast<char>(presence));
        if (presence != Presence::Visible) {
            require(pos == Coord{} && !log && !wood && !on_ground && !in_job && !reachable
                && !native_type && !quality && !wear && !other_flags && !other_refs
                && !subtype && !material && !material_index && jobs.empty()
                && ground.encode() == std::string(1, '\0'), 5);
            return out;
        }
        require(native_type <= INT32_MAX && quality <= INT32_MAX && wear <= INT32_MAX
            && other_refs <= 4096 && jobs.size() <= 8, 5);
        out += pos.encode(); flag(out, log); flag(out, wood); u32(out, native_type);
        for (auto n : {subtype, material, material_index}) i32(out, n);
        u32(out, quality); u32(out, wear); u32(out, other_flags); u32(out, other_refs);
        flag(out, on_ground); flag(out, in_job); out.push_back(static_cast<char>(jobs.size()));
        for (std::size_t i = 0; i < jobs.size(); ++i) {
            require(jobs[i] < INT32_MAX && (!i || jobs[i-1] < jobs[i]), 5); u32(out, jobs[i]);
        }
        require(ground.presence == Presence::Visible, 5); out += ground.encode(); flag(out, reachable);
        return out;
    }
    // Exactly one unclaimed, unforbidden, unhidden plant-WOOD log on dry visible floor.
    bool available() const {
        return presence == Presence::Visible && log && wood && on_ground && !in_job
            && !(other_flags & ~COMPUTED_ITEM_FLAGS) && !other_refs && jobs.empty() && wear == 0
            && material >= 0 && reachable && ground.dry() && ground.shape == 3 && !ground.occupied
            && !ground.building;
    }
};
// One living active citizen. Each field is an observed SDK predicate, not a
// scheduling certificate: `labor` is the current enabled CARPENTER labor bit,
// `labor_valid` is Units::isValidLabor (civilization permission only),
// `job_available` is Units::isJobAvailable (no current job, cage, chain,
// activity, unbailable social activity or ordered squad), and `reachable` is
// only equal nonzero walk-group connectivity to the center. Work details and
// the game's scheduler remain uncaptured; no assignment is predicted.
struct Worker {
    std::uint32_t id = 0;
    Coord pos;
    bool visible = false, adult = false, sane = false, labor = false, labor_valid = false;
    bool job_available = false, reachable = false;
    std::string encode() const {
        require(id < INT32_MAX, 5);
        std::string out; u32(out, id); out += pos.encode();
        for (bool b : {visible, adult, sane, labor, labor_valid, job_available, reachable}) flag(out, b);
        return out;
    }
    bool eligible() const { return visible && adult && sane && labor && labor_valid && job_available && reachable; }
};
struct Capture {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0, next_building = 0, next_job = 0, building_count = 0, zone_count = 0;
    std::array<std::uint32_t, 3> dimensions{};
    std::string folder;
    bool paused = false, free_footprint = false, supported = false, census_complete = false;
    Selection selection;
    std::array<Tile, 9> tiles{}; // y-major footprint; index 4 is the center.
    Item item;
    std::vector<Worker> workers;
    std::string encode() const {
        require(generation && generation < UINT64_MAX && sequence < UINT64_MAX && tick <= MAX_TICK, 5);
        require(site <= INT32_MAX && next_building <= INT32_MAX && next_job <= INT32_MAX
            && building_count <= MAX_BUILDINGS && zone_count <= building_count && utf8(folder, 512), 5);
        for (auto d : dimensions) require(d > 0 && d <= 32768, 5);
        const auto &p = selection.center;
        require(p.x + 1 < dimensions[0] && p.y + 1 < dimensions[1] && p.z < dimensions[2], 5);
        require(workers.size() <= MAX_WORKERS && (census_complete || workers.empty()), 5);
        std::string out = "DFMWC020"; u64(out, generation); u64(out, sequence); u64(out, tick);
        u32(out, site); for (auto d : dimensions) u32(out, d);
        u32(out, next_building); u32(out, next_job); u32(out, building_count); u32(out, zone_count);
        field(out, folder);
        for (bool b : {paused, free_footprint, supported, census_complete}) flag(out, b);
        out += selection.encode(); for (const auto &t : tiles) out += t.encode();
        if (item.presence == Presence::Visible)
            require(item.pos.x < dimensions[0] && item.pos.y < dimensions[1] && item.pos.z < dimensions[2], 5);
        out += item.encode(); out.push_back(static_cast<char>(workers.size()));
        for (std::size_t i = 0; i < workers.size(); ++i) {
            const auto &w = workers[i];
            require((!i || workers[i-1].id < w.id) && w.pos.x < dimensions[0]
                && w.pos.y < dimensions[1] && w.pos.z < dimensions[2], 5);
            out += w.encode();
        }
        require(out.size() <= MAX_CAPTURE_BYTES, 5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    // The selected log must lie outside the footprint. This is explicit, not
    // inferred from a native item-occupancy bit: a log inside the footprint
    // would make the exact expected-after ground tile wrong by construction.
    bool item_inside_footprint() const {
        const auto &c = selection.center, &p = item.pos;
        return item.presence == Presence::Visible && p.z == c.z && p.x + 1 >= c.x && p.x <= c.x + 1
            && p.y + 1 >= c.y && p.y <= c.y + 1;
    }
    bool eligible() const {
        (void)encode();
        if (!paused || !free_footprint || !supported || !census_complete || sequence >= UINT64_MAX - 1
            || next_building >= INT32_MAX || next_job >= INT32_MAX || building_count >= MAX_BUILDINGS
            || !item.available() || item_inside_footprint()) return false;
        for (const auto &t : tiles) if (!t.empty_floor()) return false;
        for (const auto &w : workers) if (w.eligible()) return true;
        return false;
    }
    // The exact historical stage-zero registration. Every footprint tile becomes
    // Planned and covered by the one new building; nothing else may change.
    Capture expected_after() const {
        require(eligible(), 4);
        auto out = *this; ++out.sequence; ++out.next_building; ++out.next_job; ++out.building_count;
        out.free_footprint = false;
        for (auto &t : out.tiles) { t.occupied = true; t.building = next_building; }
        out.item.in_job = true; out.item.jobs = {next_job}; return out;
    }
};
inline std::string plan_digest(const Selection &s, const std::string &witness) {
    require(witness.size() == 32); return hash("dfmcp-workshop-plan/1", s.encode() + witness);
}
inline std::string token_for(const std::string &key, const std::string &plan) {
    key_valid(key); require(plan.size() == 32);
    std::string data; field(data, key); data += plan; return hash("dfmcp-workshop-token/1", data).substr(0, 16);
}
// Independent native readback of the new building, its ConstructBuilding job,
// the exact Hauled log and reverse link. Observed constructor facts (building
// flag word, design presence, maximum stage) are recorded, not assumed.
struct Insertion {
    std::uint32_t building = 0, job = 0, item = 0, stage = 0, max_stage = 0, building_flags = 0;
    Kind kind = Kind::Carpenters;
    std::uint32_t x1 = 0, y1 = 0, x2 = 0, y2 = 0, z = 0, center_x = 0, center_y = 0;
    std::int32_t material = 0, material_index = 0;
    std::uint16_t planned_tiles = 0;
    bool has_design = false, setting_occupancy = false;
    bool linked = false, construct_job = false, exact_item_link = false, suspended = false;
    std::string encode() const {
        require(building < INT32_MAX && job < INT32_MAX && item < INT32_MAX && kind == Kind::Carpenters
            && max_stage >= 1 && max_stage <= 32 && stage <= max_stage && planned_tiles <= ALL_TILES, 5);
        std::string out = "DFMWI020"; u32(out, building); u32(out, job); u32(out, item);
        out.push_back(static_cast<char>(kind));
        for (auto n : {x1, y1, x2, y2, z, center_x, center_y}) { require(n < 32768, 5); u32(out, n); }
        i32(out, material); i32(out, material_index); u32(out, stage); u32(out, max_stage);
        u32(out, building_flags); flag(out, has_design); flag(out, setting_occupancy); u16(out, planned_tiles);
        for (bool b : {linked, construct_job, exact_item_link, suspended}) flag(out, b);
        return out;
    }
    bool matches(const Capture &c) const {
        (void)encode(); const auto &p = c.selection.center;
        return building == c.next_building && job == c.next_job && item == c.selection.item
            && kind == c.selection.kind && x1 == p.x - 1 && x2 == p.x + 1 && y1 == p.y - 1
            && y2 == p.y + 1 && z == p.z && center_x == p.x && center_y == p.y
            && material == c.item.material && material_index == c.item.material_index && stage == 0
            && setting_occupancy && planned_tiles == ALL_TILES
            && linked && construct_job && exact_item_link && !suspended;
    }
};
// Fresh read-only evidence for one historical building ID. It carries the
// subtype, material and geometry that operations/1.3-1.4 rows omit. Absence is
// an observed complete-registry fact, never interpreted as completion.
struct Progress {
    std::uint64_t generation = 0, tick = 0;
    std::uint32_t site = 0, next_building = 0, building = 0, item = 0;
    std::array<std::uint32_t, 3> dimensions{};
    std::string folder;
    bool present = false, carpenters = false, exists = false, item_contained = false;
    std::int32_t subtype = 0, custom = 0, material = 0, material_index = 0;
    std::uint32_t x1 = 0, y1 = 0, x2 = 0, y2 = 0, z = 0, center_x = 0, center_y = 0;
    std::uint32_t stage = 0, max_stage = 0, jobs = 0, construct_jobs = 0, contained = 0;
    std::string encode() const {
        require(generation && generation < UINT64_MAX && tick <= MAX_TICK && site <= INT32_MAX
            && next_building <= INT32_MAX && building < INT32_MAX && item < INT32_MAX && utf8(folder, 512), 5);
        for (auto d : dimensions) require(d > 0 && d <= 32768, 5);
        std::string out = "DFMWP020"; u64(out, generation); u64(out, tick); u32(out, site);
        for (auto d : dimensions) u32(out, d);
        field(out, folder); u32(out, next_building); u32(out, building); u32(out, item); flag(out, present);
        if (!present) {
            require(!carpenters && !exists && !item_contained && !subtype && !custom && !material
                && !material_index && !x1 && !y1 && !x2 && !y2 && !z && !center_x && !center_y
                && !stage && !max_stage && !jobs && !construct_jobs && !contained, 5);
            require(out.size() <= MAX_PROGRESS_BYTES, 5); return out;
        }
        require(building < next_building && max_stage >= 1 && max_stage <= 32 && stage <= max_stage
            && jobs <= 8 && construct_jobs <= jobs && contained <= 64 && (!item_contained || contained), 5);
        flag(out, carpenters); i32(out, subtype); i32(out, custom);
        for (auto n : {x1, y1, x2, y2, z, center_x, center_y}) { require(n < 32768, 5); u32(out, n); }
        i32(out, material); i32(out, material_index); u32(out, stage); u32(out, max_stage);
        flag(out, exists); out.push_back(static_cast<char>(jobs)); out.push_back(static_cast<char>(construct_jobs));
        out.push_back(static_cast<char>(contained)); flag(out, item_contained);
        require(out.size() <= MAX_PROGRESS_BYTES, 5); return out;
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
    std::string encode() const {
        key_valid(key); require(plan == plan_digest(before.selection, before.witness()) && token == token_for(key, plan), 5);
        require(before.eligible(), 5);
        require((phase == Phase::Prepared && reason == Reason::None)
            || (phase == Phase::Indeterminate && reason == Reason::NativeFailure)
            || (phase == Phase::Placed && reason == Reason::None)
            || (phase == Phase::Refused && (reason == Reason::Stale || reason == Reason::Expired || reason == Reason::SourceChanged))
            || (phase == Phase::Cancelled && reason == Reason::Cancelled), 5);
        require(after.has_value() == (phase == Phase::Placed) && insertion.has_value() == after.has_value(), 5);
        std::string out = "DFMWR020"; field(out, key); field(out, before.encode()); out += plan; out += token;
        out.push_back(static_cast<char>(phase)); out.push_back(static_cast<char>(reason));
        flag(out, attempted()); flag(out, after.has_value());
        if (after) {
            require(after->encode() == before.expected_after().encode() && insertion->matches(before), 5);
            field(out, after->encode()); field(out, insertion->encode());
        }
        out += hash("dfmcp-workshop-receipt/1", out); require(out.size() <= MAX_RECORD_BYTES, 5); return out;
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
    // Stateless fresh evidence for an ID; it reads, it does not authenticate.
    // The bridge generation is reported so the consumer can require exact
    // continuity with the original receipt: after a restart or reload, a
    // same-ID building is NOT evidence about the original goal.
    template<class Reader> Progress progress(std::uint32_t building, std::uint32_t item, Reader read) const {
        require(available() && building < INT32_MAX && item < INT32_MAX, 3);
        const auto g = generation_, s = sequence_;
        auto value = read(building, item);
        require(g == generation_ && s == sequence_ && value.building == building && value.item == item, 6);
        value.generation = g; (void)value.encode(); return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        key_valid(key); require(plan.size() == 32);
        const auto it = records_.find(key); if (it == records_.end()) return nullptr;
        require(it->second.plan == plan, 7); return &it->second;
    }
    // Read-only recovery of an attempted construction, never a second writer
    // call. Only the unchanged original paused frontier can discharge
    // uncertainty; every other lookup is pure retained history.
    template<class Reader, class Verifier> const Record *reconcile(const std::string &key,
        const std::string &plan, Reader read, Verifier verify) {
        Busy owned(busy_);
        const auto *known = query(key, plan);
        if (!known || known->phase != Phase::Indeterminate) return known;
        auto &r = records_.find(key)->second;
        if (!unresolved_ || !available() || generation_ != r.before.generation
            || sequence_ != r.before.sequence + 1) return &r;
        try {
            const auto expected = r.before.expected_after().encode();
            verify_after(r, expected, read, verify);
        } catch (...) { /* Allocation/read failures preserve the original uncertain record. */ }
        return &r;
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
        // Never deconstruct, detach or cancel anything after any attempt.
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
        verify_after(r, expected, read, verify);
        return r;
    }
private:
    // Commit and reconciliation use the same complete, double-captured proof.
    template<class Reader, class Verifier> void verify_after(Record &r,
        const std::string &expected, Reader read, Verifier verify) {
        try {
            const auto after = inspect(r.before.selection, read);
            if (after.encode() != expected) return;
            const auto proof = verify(r.before);
            if (!proof.matches(r.before) || inspect(r.before.selection, read).encode() != expected) return;
            Record next = r; next.after = after; next.insertion = proof;
            next.phase = Phase::Placed; next.reason = Reason::None;
            (void)next.encode(); r = std::move(next); unresolved_ = false;
        } catch (...) { /* No rollback, no retry, no invented nonapplication. */ }
    }
    std::uint64_t generation_, sequence_ = 0;
    bool unresolved_ = false, busy_ = false;
    std::map<std::string, Record> records_;
    Record &authenticated(const std::string &key, const std::string &plan, const std::string &token) {
        const auto *r = query(key, plan); require(r && token.size() == 16 && r->token == token, 7);
        return records_.find(key)->second;
    }
};
} // namespace dfmcp_workshop
