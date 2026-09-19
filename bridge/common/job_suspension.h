#pragma once
#include "retained_snapshot.h"
#include <cstdint>
#include <exception>
#include <limits>
#include <map>
#include <string>
#include <utility>

// A fixed boolean job-setting transaction, not a generic native command engine.
// The caller holds DFHack's suspension throughout every reader/writer callback.
// Records retain data only; no job, worker or building pointer survives a call.
namespace dfmcp_job_suspension {
using Clock = std::chrono::steady_clock;
constexpr std::size_t MAX_RECORDS = 4096;
constexpr std::chrono::seconds PREPARE_LIFETIME{60};
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t value) : code(value) {}
    const char *what() const noexcept override { return "job suspension refused"; }
};
inline void require(bool value, std::uint32_t code = 3) { if (!value) throw Failure(code); }
inline void u32(std::string &out, std::uint32_t n) {
    for (int s = 24; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void u64(std::string &out, std::uint64_t n) {
    for (int s = 56; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void text(std::string &out, const std::string &s) {
    out.push_back(static_cast<char>(s.size() >> 8));
    out.push_back(static_cast<char>(s.size())); out += s;
}
inline bool utf8(const std::string &s, std::size_t maximum, bool empty = false) {
    if (s.size() > maximum || (!empty && s.empty())) return false;
    for (std::size_t i = 0; i < s.size();) {
        const auto c = static_cast<unsigned char>(s[i]);
        if (!c) return false;
        if (c < 128) { ++i; continue; }
        const std::size_t n = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
        if (!n || s.size() - i < n) return false;
        for (std::size_t k = 1; k < n; ++k)
            if ((static_cast<unsigned char>(s[i+k]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(s[i+1]);
        if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0)
            || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void valid_key(const std::string &key) {
    require(!key.empty() && key.size() <= 128);
    for (unsigned char c : key)
        require((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
            || (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.');
}
inline std::string domain(const char *value) { std::string out(value); out.push_back('\0'); return out; }
inline std::string plan_digest(std::uint32_t job, bool desired, const std::string &witness) {
    require(job <= INT32_MAX && witness.size() == 32);
    auto bytes = domain("dfmcp-job-suspension-plan/1"); u32(bytes, job);
    bytes.push_back(desired ? 1 : 0); bytes += witness;
    return dfmcp_snapshot::sha256(bytes);
}
inline std::string prepare_token(std::uint64_t generation, const std::string &key, const std::string &plan) {
    auto bytes = domain("dfmcp-job-suspension-token/1"); u64(bytes, generation);
    text(bytes, key); bytes += plan; return dfmcp_snapshot::sha256(bytes).substr(0, 16);
}

struct Observation {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t job = 0, next_job = 0, site = 0, attachments = 0, filters = 0;
    std::int32_t type = -1, holder = -1, holder_type = -1, worker = -1;
    std::int32_t x = 0, y = 0, z = 0, timer = -1;
    bool suspended = false, repeating = false, paused = false;
    bool holder_complete = false, supported = false, production_holder = false;
    std::string folder, type_key, reaction;
    bool eligible() const {
        return paused && holder >= 0 && production_holder && holder_complete
            && supported && worker == -1 && timer == -1;
    }
    std::string encode() const {
        require(generation && generation != UINT64_MAX && sequence != UINT64_MAX, 5);
        require(job < next_job && next_job <= INT32_MAX && site <= INT32_MAX && type >= 0
            && holder >= -1 && holder_type >= -1 && worker >= -1 && timer >= -1
            && attachments <= 65536 && filters <= 4096, 5);
        require(tick <= std::uint64_t{UINT32_MAX} * 403200 + 403199
            && utf8(folder, 512) && utf8(type_key, 128) && utf8(reaction, 128, true), 5);
        std::string out = "DFMJS019"; u64(out, generation); u64(out, sequence); u64(out, tick);
        for (auto n : {job, next_job, site}) u32(out, n);
        for (auto n : {type, holder, holder_type, worker, x, y, z, timer}) u32(out, static_cast<std::uint32_t>(n));
        u32(out, attachments); u32(out, filters);
        out.push_back(static_cast<char>((suspended ? 1 : 0) | (repeating ? 2 : 0) | (paused ? 4 : 0)
            | (holder_complete ? 8 : 0) | (supported ? 16 : 0) | (production_holder ? 32 : 0)));
        text(out, folder); text(out, type_key); text(out, reaction); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
};
enum class State : unsigned char { Prepared = 0, Unknown = 1, Applied = 2, NotApplied = 3, Refused = 4 };
struct Record {
    std::string key, plan, witness, token;
    Observation before;
    bool desired = false;
    State state = State::Prepared;
    Clock::time_point created;
    bool after_known = false, after_suspended = false;
    std::uint64_t after_tick = 0;
    std::string after_witness = std::string(32, '\0'), receipt = std::string(32, '\0');
    bool terminal() const { return state == State::Applied || state == State::NotApplied || state == State::Refused; }
    std::string proof() const {
        auto out = domain("dfmcp-job-suspension-receipt/1"); u64(out, before.generation);
        text(out, key); out += plan; out += token; out.push_back(static_cast<char>(state));
        out.push_back(after_known ? 1 : 0); out.push_back(after_suspended ? 1 : 0);
        u64(out, after_tick); out += after_witness; return dfmcp_snapshot::sha256(out);
    }
    std::string encode() const {
        std::string out = "DFMJSE19"; u64(out, before.generation); u64(out, before.sequence);
        u64(out, before.tick); u32(out, before.job); out.push_back(desired ? 1 : 0);
        out += witness; out += plan; out += token; out.push_back(static_cast<char>(state));
        out.push_back(after_known ? 1 : 0); out.push_back(after_suspended ? 1 : 0);
        u64(out, after_tick); out += after_witness; out += receipt; text(out, key); return out;
    }
};
class Engine {
public:
    explicit Engine(std::uint64_t generation) : generation_(generation) {}
    std::uint64_t generation() const { return generation_; }
    std::uint64_t sequence() const { return sequence_; }
    std::size_t size() const { return records_.size(); }
    bool available() const { return generation_ && generation_ != UINT64_MAX && sequence_ != UINT64_MAX; }
    void interrupt() { if (sequence_ != UINT64_MAX) ++sequence_; }
    void reset() { records_.clear(); if (generation_ != UINT64_MAX) ++generation_; interrupt(); }

    template<class Reader> Observation inspect(std::uint32_t id, Reader read) const {
        require(available(), 5); require(id <= INT32_MAX);
        auto value = read(id); value.generation = generation_; value.sequence = sequence_;
        require(value.job == id, 5); (void)value.encode(); return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        valid_key(key); require(plan.size() == 32);
        const auto it = records_.find(key); if (it == records_.end()) return nullptr;
        require(it->second.plan == plan, 7); return &it->second;
    }
    template<class Reader> std::pair<const Record *, bool> prepare(const std::string &key, std::uint32_t id,
        bool desired, const std::string &witness, const std::string &plan, Clock::time_point now, Reader read) {
        valid_key(key); require(plan == plan_digest(id, desired, witness), 7);
        if (const auto *old = query(key, plan)) return {old, true};
        require(records_.size() < MAX_RECORDS && available());
        auto before = inspect(id, read); require(before.eligible(), 4); require(before.witness() == witness, 6);
        Record r; r.key = key; r.plan = plan; r.witness = witness; r.before = std::move(before);
        r.desired = desired; r.created = now; r.token = prepare_token(generation_, key, plan);
        const auto stored = records_.emplace(key, std::move(r)); return {&stored.first->second, false};
    }
    template<class Reader, class Writer> const Record &commit(const std::string &key, const std::string &plan,
        const std::string &token, Clock::time_point now, Reader read, Writer write) {
        const auto *known = query(key, plan); require(known && token.size() == 16 && known->token == token, 7);
        auto &r = records_.find(key)->second;
        if (r.state != State::Prepared) return r; // Never re-dispatch, including Unknown and Refused.
        const auto latest = Clock::time_point::max() - PREPARE_LIFETIME;
        const bool fresh = now >= r.created && (r.created > latest || now < r.created + PREPARE_LIFETIME);
        bool valid = fresh && available() && r.before.sequence == sequence_ && r.before.generation == generation_;
        try {
            if (valid) { const auto current = inspect(r.before.job, read);
                valid = current.eligible() && current.witness() == r.witness; }
        } catch (...) { valid = false; }
        if (!valid) {
            // Receipt construction may allocate. Retire first so even an OOM
            // cannot make this old preparation dispatchable on a later call.
            r.state = State::Refused;
            r.receipt = r.proof(); return r;
        }
        // Fence competing preparations and retain unknown outcome BEFORE the sole setter.
        interrupt(); r.state = State::Unknown;
        try {
            write(r.before.job, r.desired);
            auto after = inspect(r.before.job, read);
            // The controlled flag and local sequence are the only allowed changes.
            auto comparable = after; comparable.sequence = r.before.sequence;
            comparable.suspended = r.before.suspended;
            if (comparable.encode() != r.before.encode()) return r;
            auto next = r; next.state = after.suspended == r.desired ? State::Applied : State::NotApplied;
            next.after_known = true; next.after_suspended = after.suspended; next.after_tick = after.tick;
            next.after_witness = after.witness(); next.receipt = next.proof();
            r = std::move(next); // Publish terminal evidence only after its construction succeeds.
        } catch (...) { /* The setter may have run. Retain Unknown, never replay it. */ }
        return r;
    }
private:
    std::uint64_t generation_, sequence_ = 0;
    std::map<std::string, Record> records_;
};
} // namespace dfmcp_job_suspension
