#pragma once
#include "retained_snapshot.h"
#include <exception>
#include <vector>

// Isolated work-orders/1.10. All callbacks execute in one DFHack suspension.
// The queue witness proves IDs/horizon, NOT existing-order contents or resources.
// Records contain values only. Native retention is not a durable coordinator.
namespace dfmcp_work_orders {
using Clock = std::chrono::steady_clock;
constexpr std::size_t MAX_ORDERS = 4096, MAX_RECORDS = 256;
constexpr std::size_t MAX_OBSERVATION_BYTES = 17 * 1024, MAX_EFFECT_BYTES = 357;
constexpr std::uint32_t MAX_AMOUNT = 100;
constexpr std::chrono::seconds PREPARE_LIFETIME{60};
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t value) : code(value) {}
    const char *what() const noexcept override { return "work order refused"; }
};
inline void require(bool value, std::uint32_t code = 3) { if (!value) throw Failure(code); }
inline void u32(std::string &out, std::uint32_t n) {
    for (int s = 24; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void u64(std::string &out, std::uint64_t n) {
    for (int s = 56; s >= 0; s -= 8) out.push_back(static_cast<char>(n >> s));
}
inline void text(std::string &out, const std::string &value) {
    require(value.size() <= UINT16_MAX);
    out.push_back(static_cast<char>(value.size() >> 8));
    out.push_back(static_cast<char>(value.size())); out += value;
}
inline bool utf8(const std::string &s, std::size_t maximum) {
    if (s.empty() || s.size() > maximum) return false;
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

// Stable semantic codes, never client-selected DF job enum numbers or reactions.
// Each creates a global, OneTime, wood-only order, max_workshops=1, with no
// conditions, item filters, art/specification, validation or activation override.
enum class Recipe : unsigned char { WoodenBed = 1, WoodenDoor = 2, WoodenTable = 3, WoodenChair = 4 };
struct Spec {
    Recipe recipe;
    std::uint32_t amount;
    void validate() const {
        const auto value = static_cast<unsigned>(recipe);
        require(value >= 1 && value <= 4 && amount >= 1 && amount <= MAX_AMOUNT);
    }
    std::string encode() const { validate(); std::string out(1, static_cast<char>(recipe)); u32(out, amount); return out; }
};
struct Observation {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t next_order = 0, site = 0;
    bool paused = false;
    std::string folder;
    std::vector<std::uint32_t> ids;
    bool eligible() const {
        return paused && ids.size() < MAX_ORDERS && next_order < INT32_MAX && sequence < UINT64_MAX - 1;
    }
    std::string encode() const {
        require(generation && generation != UINT64_MAX && sequence != UINT64_MAX, 5);
        require(tick <= std::uint64_t{UINT32_MAX} * 403200 + 403199
            && next_order <= INT32_MAX && site <= INT32_MAX && utf8(folder, 512) && ids.size() <= MAX_ORDERS, 5);
        std::string out = "DFMWO010"; u64(out, generation); u64(out, sequence); u64(out, tick);
        u32(out, next_order); u32(out, site); out.push_back(paused ? 1 : 0); text(out, folder);
        u32(out, static_cast<std::uint32_t>(ids.size()));
        for (std::size_t i = 0; i < ids.size(); ++i) {
            require(ids[i] < next_order && (i == 0 || ids[i-1] < ids[i]), 5);
            u32(out, ids[i]);
        }
        require(out.size() <= MAX_OBSERVATION_BYTES, 5); return out;
    }
    std::string witness() const { return dfmcp_snapshot::sha256(encode()); }
    Observation expected_after() const {
        (void)encode(); require(eligible(), 4);
        auto out = *this; out.ids.push_back(next_order); ++out.next_order; ++out.sequence; return out;
    }
};
// Emitted by the native verifier ONLY after checking every template field.
inline std::string configuration(std::uint32_t id, Spec spec) {
    require(id < INT32_MAX); std::string out = "DFMWOC10"; u32(out, id); out += spec.encode(); return out;
}
inline std::string plan_digest(Spec spec, const std::string &witness) {
    require(witness.size() == 32); auto bytes = domain("dfmcp-work-order-plan/1");
    bytes += spec.encode(); bytes += witness; return dfmcp_snapshot::sha256(bytes);
}
inline std::string prepare_token(std::uint64_t generation, const std::string &key, const std::string &plan) {
    auto bytes = domain("dfmcp-work-order-token/1"); u64(bytes, generation);
    text(bytes, key); bytes += plan; return dfmcp_snapshot::sha256(bytes).substr(0, 16);
}
enum class State : unsigned char { Prepared = 0, Unknown = 1, Created = 2, Refused = 4 };
struct Record {
    std::string key, plan, witness, token;
    Observation before;
    Spec spec{Recipe::WoodenBed, 1};
    State state = State::Prepared;
    Clock::time_point created;
    bool after_known = false;
    std::uint64_t after_tick = 0;
    std::string after_witness = std::string(32, '\0'), configuration_witness = std::string(32, '\0');
    std::string receipt = std::string(32, '\0');
    bool terminal() const { return state == State::Created || state == State::Refused; }
    std::string proof() const {
        auto out = domain("dfmcp-work-order-receipt/1"); u64(out, before.generation);
        text(out, key); out += plan; out += token; out.push_back(static_cast<char>(state));
        out.push_back(after_known ? 1 : 0); u64(out, after_tick);
        out += after_witness; out += configuration_witness; return dfmcp_snapshot::sha256(out);
    }
    std::string encode() const {
        std::string out = "DFMWOE10"; u64(out, before.generation); u64(out, before.sequence);
        u64(out, before.tick); u32(out, before.next_order); out += spec.encode();
        out += witness; out += plan; out += token; out.push_back(static_cast<char>(state));
        out.push_back(after_known ? 1 : 0); u64(out, after_tick);
        out += after_witness; out += configuration_witness; out += receipt; text(out, key);
        require(out.size() <= MAX_EFFECT_BYTES, 5); return out;
    }
};
class Engine {
public:
    explicit Engine(std::uint64_t generation) : generation_(generation) {}
    std::uint64_t generation() const { return generation_; }
    std::uint64_t sequence() const { return sequence_; }
    std::size_t size() const { return records_.size(); }
    bool available() const { return generation_ && generation_ != UINT64_MAX && sequence_ != UINT64_MAX; }
    bool unresolved() const { return unresolved_; }
    void interrupt() { if (sequence_ != UINT64_MAX) ++sequence_; }
    void reset() {
        records_.clear(); unresolved_ = false;
        if (generation_ != UINT64_MAX) ++generation_;
        interrupt();
    }
    template<class Reader> Observation inspect(Reader read) const {
        require(available(), 5); auto value = read(); value.generation = generation_; value.sequence = sequence_;
        (void)value.encode(); return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        valid_key(key); require(plan.size() == 32);
        const auto it = records_.find(key); if (it == records_.end()) return nullptr;
        require(it->second.plan == plan, 7); return &it->second;
    }
    template<class Reader> std::pair<const Record *, bool> prepare(const std::string &key, Spec spec,
        const std::string &witness, const std::string &plan, Clock::time_point now, Reader read) {
        valid_key(key); require(plan == plan_digest(spec, witness), 7);
        if (const auto *old = query(key, plan)) return {old, true};
        require(!unresolved_, 8); require(records_.size() < MAX_RECORDS && available(), 5);
        auto before = inspect(read); require(before.eligible(), 4); require(before.witness() == witness, 6);
        Record r; r.key = key; r.plan = plan; r.witness = witness; r.before = std::move(before);
        r.spec = spec; r.created = now; r.token = prepare_token(generation_, key, plan);
        const auto stored = records_.emplace(key, std::move(r)); return {&stored.first->second, false};
    }
    template<class Reader, class Writer, class Verify> const Record &commit(const std::string &key,
        const std::string &plan, const std::string &token, Clock::time_point now, Reader read, Writer write, Verify verify) {
        const auto *known = query(key, plan); require(known && token.size() == 16 && known->token == token, 7);
        auto &r = records_.find(key)->second;
        if (r.state != State::Prepared) return r; // Never repeat a created, refused or uncertain insertion.
        require(!unresolved_, 8);
        const auto latest = Clock::time_point::max() - PREPARE_LIFETIME;
        const bool fresh = now >= r.created && (r.created > latest || now < r.created + PREPARE_LIFETIME);
        bool valid = fresh && available() && r.before.sequence == sequence_ && r.before.generation == generation_;
        try {
            if (valid) { const auto current = inspect(read);
                valid = current.eligible() && current.witness() == r.witness; }
        } catch (...) { valid = false; }
        if (!valid) {
            r.state = State::Refused; // Retire BEFORE receipt allocation, even if allocation throws.
            r.receipt = r.proof(); return r;
        }
        // Compute the complete expected readback before the mutation boundary.
        const auto expected = r.before.expected_after().encode();
        const auto config = configuration(r.before.next_order, r.spec);
        interrupt(); r.state = State::Unknown; unresolved_ = true;
        // An insertion can succeed before its writer throws. Recover only from
        // immediate native evidence under the same suspension, never by retrying
        // creation or by treating an unchanged queue as proof of non-creation.
        try { write(r.before.next_order, r.spec); }
        catch (...) { /* The complete queue and configuration readback decide. */ }
        try {
            const auto after = inspect(read);
            if (after.encode() != expected || verify(r.before.next_order, r.spec) != config) return r;
            // Configuration verification is another native callback. Bind its
            // result to the same queue, clock and mutation sequence before we
            // publish Created and release the unresolved-creation guard.
            if (inspect(read).encode() != expected) return r;
            auto next = r; next.state = State::Created; next.after_known = true; next.after_tick = after.tick;
            next.after_witness = after.witness(); next.configuration_witness = dfmcp_snapshot::sha256(config);
            next.receipt = next.proof(); r = std::move(next); unresolved_ = false;
        } catch (...) { /* Insertion may have happened. No retry, rollback, or fabricated negative receipt. */ }
        return r;
    }
private:
    std::uint64_t generation_, sequence_ = 0;
    bool unresolved_ = false;
    std::map<std::string, Record> records_;
};
} // namespace dfmcp_work_orders
