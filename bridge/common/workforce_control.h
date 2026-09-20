#pragma once
#include "retained_snapshot.h"
#include <exception>
#include <set>
#include <vector>

// Values only. Every caller holds one DFHack suspension through capture/write/
// readback. Membership is control configuration, not a job assignment or proof
// of future work. No C++ pointers or ambient authority cross this boundary.
namespace dfmcp_workforce {
using Clock = std::chrono::steady_clock;
constexpr std::size_t MAX_UNITS = 32, MAX_DETAILS = 64, MAX_MEMBERS = 4096;
constexpr std::size_t MAX_LABORS = 128, MAX_RECORDS = 64, MAX_CAPTURE = 65536;
constexpr std::size_t MAX_EFFECT = 8192;
constexpr std::uint64_t MAX_TICK = std::uint64_t{UINT32_MAX} * 403200 + 403199;
constexpr std::chrono::seconds LIFETIME{60};
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t n) : code(n) {}
    const char *what() const noexcept override { return "workforce operation refused"; }
};
inline void require(bool v, std::uint32_t code = 3) { if (!v) throw Failure(code); }
inline void u16(std::string &s, std::size_t n) {
    require(n <= UINT16_MAX); s.push_back(static_cast<char>(n >> 8)); s.push_back(static_cast<char>(n));
}
inline void u32(std::string &s, std::uint32_t n) {
    for (int i = 24; i >= 0; i -= 8) s.push_back(static_cast<char>(n >> i));
}
inline void u64(std::string &s, std::uint64_t n) {
    for (int i = 56; i >= 0; i -= 8) s.push_back(static_cast<char>(n >> i));
}
inline void field(std::string &s, const std::string &v) { u16(s, v.size()); s += v; }
inline std::string hash(const char *domain, const std::string &s) {
    auto bytes = std::string(domain); bytes.push_back('\0'); bytes += s; return dfmcp_snapshot::sha256(bytes);
}
inline bool utf8(const std::string &s, std::size_t maximum, bool empty = false) {
    if (s.size() > maximum || (!empty && s.empty())) return false;
    for (std::size_t i = 0; i < s.size();) {
        const auto a = static_cast<unsigned char>(s[i]); if (!a) return false;
        if (a < 128) { ++i; continue; }
        const std::size_t n = a >= 0xc2 && a <= 0xdf ? 2 : a >= 0xe0 && a <= 0xef ? 3 : a >= 0xf0 && a <= 0xf4 ? 4 : 0;
        if (!n || n > s.size() - i) return false;
        for (std::size_t j = 1; j < n; ++j)
            if ((static_cast<unsigned char>(s[i+j]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(s[i+1]);
        if ((a == 0xe0 && b < 0xa0) || (a == 0xed && b >= 0xa0)
            || (a == 0xf0 && b < 0x90) || (a == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void key_ok(const std::string &key) {
    require(!key.empty() && key.size() <= 128);
    for (unsigned char c : key)
        require((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')
            || c == '_' || c == '.' || c == '-');
}
inline void ids_ok(const std::vector<std::uint32_t> &ids, std::size_t limit, bool empty = false) {
    require((empty || !ids.empty()) && ids.size() <= limit);
    for (std::size_t i = 0; i < ids.size(); ++i)
        require(ids[i] <= INT32_MAX && (!i || ids[i-1] < ids[i]));
}
inline void bits_ok(const std::string &bits, std::size_t n) {
    require(bits.size() == n); for (unsigned char b : bits) require(b <= 1);
}
struct Detail {
    std::string name, labors;
    std::uint32_t flags = 0;
    bool selected_only = false;
    std::vector<std::uint32_t> members;
    std::string encode(std::size_t n) const {
        require(utf8(name, 256, true)); bits_ok(labors, n); ids_ok(members, MAX_MEMBERS, true);
        std::string out; field(out, name); u32(out, flags); out.push_back(selected_only ? 1 : 0);
        out += labors; u16(out, members.size()); for (auto id : members) u32(out, id); return out;
    }
};
struct Unit {
    std::uint32_t id = 0, historical_id = 0;
    bool eligible = false;
    std::string labors;
    std::string encode(std::size_t n) const {
        require(id <= INT32_MAX && historical_id <= INT32_MAX); bits_ok(labors, n);
        std::string out; u32(out, id); u32(out, historical_id); out.push_back(eligible ? 1 : 0); out += labors; return out;
    }
};
struct Capture {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0;
    bool paused = false, automatic = false;
    std::string folder;
    std::vector<std::string> labor_keys;
    std::vector<Detail> details;
    std::vector<Unit> units;
    std::vector<std::uint32_t> ids() const {
        std::vector<std::uint32_t> out; for (const auto &u : units) out.push_back(u.id); return out;
    }
    std::string encode() const {
        require(generation && generation != UINT64_MAX && tick <= MAX_TICK && site <= INT32_MAX
            && utf8(folder, 512) && !labor_keys.empty() && labor_keys.size() <= MAX_LABORS
            && details.size() <= MAX_DETAILS, 5);
        ids_ok(ids(), MAX_UNITS);
        std::set<std::string> keys;
        for (const auto &k : labor_keys) { require(utf8(k, 64), 5); require(keys.insert(k).second, 5); }
        std::string out = "DFMWF017";
        for (auto v : {generation, sequence, tick}) u64(out, v);
        u32(out, site); field(out, folder); out.push_back(paused ? 1 : 0); out.push_back(automatic ? 1 : 0);
        u16(out, labor_keys.size()); for (const auto &k : labor_keys) field(out, k);
        u16(out, details.size()); std::size_t members = 0;
        for (const auto &d : details) {
            members += d.members.size(); require(members <= MAX_MEMBERS, 5); out += d.encode(labor_keys.size());
        }
        u16(out, units.size()); for (const auto &u : units) out += u.encode(labor_keys.size());
        require(out.size() <= MAX_CAPTURE, 5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
};
struct Spec {
    std::uint32_t detail = 0;
    bool assigned = false;
    std::string encode() const { std::string out; u32(out, detail); out.push_back(assigned ? 1 : 0); return out; }
};
inline std::string plan_digest(Spec spec, const std::string &witness) {
    require(witness.size() == 32); return hash("dfmcp-workforce-plan/1", spec.encode() + witness);
}
inline std::string token_for(const std::string &key, const std::string &plan) {
    key_ok(key); require(plan.size() == 32); std::string input; field(input, key); input += plan;
    return hash("dfmcp-workforce-token/1", input).substr(0, 16);
}
inline std::vector<std::uint32_t> changed_ids(const Capture &before, Spec spec) {
    require(spec.detail < before.details.size()); const auto &d = before.details[spec.detail];
    std::vector<std::uint32_t> out;
    for (auto id : before.ids())
        if (std::binary_search(d.members.begin(), d.members.end(), id) != spec.assigned) out.push_back(id);
    return out;
}
// The actual recomputation may change any labor permission for changed units.
// Everything else represented by this capture must retain its exact value.
inline Capture expected_membership(const Capture &before, Spec spec) {
    (void)before.encode();
    require(before.paused && before.automatic && before.sequence < UINT64_MAX
        && spec.detail < before.details.size(), 4);
    const auto &d = before.details[spec.detail];
    require(d.selected_only && d.labors.find('\1') != std::string::npos, 4);
    for (const auto &u : before.units) require(u.eligible, 4);
    require(!changed_ids(before, spec).empty(), 4); // Do not recompute under a no-op request.
    auto expected = before; ++expected.sequence;
    auto &members = expected.details[spec.detail].members;
    for (auto id : before.ids()) {
        const auto it = std::lower_bound(members.begin(), members.end(), id);
        if (spec.assigned) { if (it == members.end() || *it != id) members.insert(it, id); }
        else if (it != members.end() && *it == id) members.erase(it);
    }
    (void)expected.encode(); return expected;
}
inline bool verify_after(const Capture &before, Spec spec, const Capture &after) {
    auto expected = expected_membership(before, spec); (void)after.encode();
    if (after.units.size() != before.units.size()) return false;
    const auto changed = changed_ids(before, spec);
    for (std::size_t i = 0; i < after.units.size(); ++i) {
        if (!std::binary_search(changed.begin(), changed.end(), before.units[i].id)) continue;
        // Adding to a selected-only detail must actually enable its labor bits.
        // Removing membership does not imply disabled bits: other details may
        // also grant them. Expose all observed post-recompute bits instead.
        if (spec.assigned)
            for (std::size_t j = 0; j < before.labor_keys.size(); ++j)
                if (before.details[spec.detail].labors[j] && !after.units[i].labors[j]) return false;
        expected.units[i].labors = after.units[i].labors;
    }
    return after.encode() == expected.encode();
}
enum class Phase : unsigned char { Prepared = 0, Unknown = 1, Applied = 2, Refused = 3, Cancelled = 4 };
struct Record {
    std::string key, plan, token, witness;
    Capture before;
    Spec spec;
    Phase phase = Phase::Prepared;
    Clock::time_point prepared{};
    std::string after_witness = std::string(32, '\0');
    std::vector<Unit> after_units;
    std::string encode() const {
        std::string out = "DFMWE017"; field(out, key); out += plan; out += token; out += spec.encode(); out += witness;
        for (auto v : {before.generation, before.sequence, before.tick}) u64(out, v);
        out.push_back(static_cast<char>(phase)); out += after_witness;
        u16(out, before.labor_keys.size()); u16(out, after_units.size());
        for (const auto &u : after_units) out += u.encode(before.labor_keys.size());
        out += hash("dfmcp-workforce-receipt/1", out); require(out.size() <= MAX_EFFECT, 5); return out;
    }
};
class Engine {
    std::uint64_t generation_, sequence_ = 0;
    bool unresolved_ = false;
    std::map<std::string, Record> records_;
public:
    explicit Engine(std::uint64_t generation) : generation_(generation) {}
    std::uint64_t generation() const { return generation_; }
    std::size_t size() const { return records_.size(); }
    bool unresolved() const { return unresolved_; }
    void reset() {
        if (generation_ != UINT64_MAX) ++generation_;
        if (sequence_ != UINT64_MAX) ++sequence_;
        records_.clear(); unresolved_ = false;
    }
    template<class Reader> Capture observe(const std::vector<std::uint32_t> &ids, Reader read) const {
        ids_ok(ids, MAX_UNITS); auto value = read(ids); value.generation = generation_; value.sequence = sequence_;
        require(value.ids() == ids, 5); (void)value.encode(); return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        key_ok(key); require(plan.size() == 32); const auto it = records_.find(key);
        if (it == records_.end()) return nullptr;
        require(it->second.plan == plan, 7); return &it->second;
    }
    template<class Reader> const Record &prepare(const std::string &key, Spec spec,
        const std::vector<std::uint32_t> &ids, const std::string &witness, const std::string &plan,
        Clock::time_point now, Reader read) {
        key_ok(key); ids_ok(ids, MAX_UNITS); require(plan == plan_digest(spec, witness), 7);
        if (const auto *old = query(key, plan)) { require(old->before.ids() == ids, 7); return *old; }
        require(!unresolved_, 8); require(records_.size() < MAX_RECORDS, 5);
        auto before = observe(ids, read); require(before.witness() == witness, 6);
        (void)expected_membership(before, spec);
        require(now <= Clock::time_point::max() - LIFETIME, 5);
        Record r; r.key = key; r.plan = plan; r.token = token_for(key, plan); r.witness = witness;
        r.before = std::move(before); r.spec = spec; r.prepared = now;
        return records_.emplace(key, std::move(r)).first->second;
    }
    template<class Reader, class Writer> const Record &commit(const std::string &key,
        const std::string &plan, const std::string &token, Clock::time_point now, Reader read, Writer write) {
        const auto *known = query(key, plan); require(known && known->token == token, 7);
        auto &r = records_.find(key)->second;
        if (r.phase != Phase::Prepared) return r;
        require(!unresolved_, 8);
        bool valid = now >= r.prepared && now < r.prepared + LIFETIME;
        try { if (valid) valid = observe(r.before.ids(), read).encode() == r.before.encode(); }
        catch (...) { valid = false; }
        if (!valid) { r.phase = Phase::Refused; return r; }
        // Allocate the complete expected membership and change list first.
        const auto expected = expected_membership(r.before, r.spec);
        const auto changed = changed_ids(r.before, r.spec);
        ++sequence_; r.phase = Phase::Unknown; unresolved_ = true; // Before first write.
        try {
            write(r.before, r.spec, expected.details[r.spec.detail].members, changed);
            const auto after = observe(r.before.ids(), read);
            if (!verify_after(r.before, r.spec, after)) return r;
            auto next = r; next.after_witness = after.witness(); next.after_units = after.units;
            next.phase = Phase::Applied; (void)next.encode();
            r = std::move(next); unresolved_ = false;
        } catch (...) { /* Partial membership/recompute is unknown, never retried or rolled back. */ }
        return r;
    }
    const Record &cancel(const std::string &key, const std::string &plan, const std::string &token) {
        const auto *known = query(key, plan); require(known && known->token == token, 7);
        auto &r = records_.find(key)->second;
        if (r.phase == Phase::Prepared) r.phase = Phase::Cancelled;
        return r; // Unknown remains unknown; cancellation never reverses membership.
    }
};
} // namespace dfmcp_workforce
