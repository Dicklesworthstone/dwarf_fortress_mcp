#pragma once
#include <cstdint>
#include <map>
#include <string>
#include <utility>
// Closed typed models supply evidence; this engine supplies one-attempt state only.
namespace dfmcp_one_attempt {
template<class Model> class Engine {
    using Capture = typename Model::Capture;
    using Selection = typename Model::Selection;
    using Progress = typename Model::Progress;
    using Record = typename Model::Record;
    using Phase = typename Model::Phase;
    using Reason = typename Model::Reason;
    static void require(bool v, std::uint32_t c = 3) { Model::require(v, c); }
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
        Model::key_valid(key); require(plan.size() == 32);
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
        Busy owned(busy_); Model::key_valid(key); require(plan == Model::plan_digest(selection, witness), 7);
        if (const auto *old = query(key, plan)) return *old; // Replay cannot renew preparation.
        require(!unresolved_, 8); require(records_.size() < Model::MAX_RECORDS, 5);
        auto before = inspect(selection, read); require(before.eligible(), 4); require(before.witness() == witness, 6);
        Record r; r.key = key; r.plan = plan; r.token = Model::token_for(key, plan); r.before = std::move(before); r.created_ms = now_ms;
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
        else if (now_ms < r.created_ms || now_ms - r.created_ms >= Model::PREPARE_MS) refusal = Reason::Expired;
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
} // namespace dfmcp_one_attempt
