#pragma once

#include <chrono>
#include <cstdint>
#include <exception>
#include <limits>
#include <map>
#include <string>
#include <utility>

// A single native clock owner, serviced by DFHack's update callback even after
// its RPC client disconnects. Callers serialize every entry under CoreSuspender.
// The writer MUST reject a generation mismatch before touching the game's clock.
// This does not fence UI input, other plugins, or another DFHack process.
namespace dfmcp_bounded_run {
using Clock = std::chrono::steady_clock;
constexpr std::uint32_t MAX_GAME_TICKS = 1200;
constexpr std::uint32_t MAX_WALL_MS = 60000;
constexpr std::size_t MAX_RECORDS = 256;
constexpr std::chrono::seconds PREPARE_LIFETIME{60};

struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t value) : code(value) {}
    const char *what() const noexcept override { return "bounded run refused"; }
};
inline void require(bool value, std::uint32_t code = 3) {
    if (!value) throw Failure(code);
}
inline void valid_key(const std::string &key) {
    require(!key.empty() && key.size() <= 128);
    for (unsigned char c : key)
        require((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
            || (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.');
}
struct Spec {
    std::uint32_t game_ticks = 0, wall_ms = 0;
    void validate() const {
        require(game_ticks >= 1 && game_ticks <= MAX_GAME_TICKS
            && wall_ms >= 1 && wall_ms <= MAX_WALL_MS);
    }
    bool operator==(const Spec &other) const {
        return game_ticks == other.game_ticks && wall_ms == other.wall_ms;
    }
};
struct Snapshot {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    bool loaded = false, clock_valid = false, paused = false;
    bool usable() const {
        return loaded && clock_valid && generation && generation != UINT64_MAX;
    }
    bool operator==(const Snapshot &other) const {
        return generation == other.generation && sequence == other.sequence
            && tick == other.tick && loaded == other.loaded
            && clock_valid == other.clock_valid && paused == other.paused;
    }
};
enum class Phase : unsigned char {
    Prepared = 0, Running = 1, Stopping = 2, Stopped = 3, Refused = 4, SourceLost = 5
};
enum class Reason : unsigned char {
    None = 0, TickLimit = 1, WallLimit = 2, Cancelled = 3, ExternalPause = 4,
    NativeFailure = 5, ClockRegression = 6, SourceChanged = 7, Shutdown = 8, Stale = 9
};
struct Record {
    std::string key, plan;
    Spec spec;
    Snapshot before;
    Phase phase = Phase::Prepared;
    Reason reason = Reason::None;
    bool unpause_attempted = false, pause_verified = false, tick_known = false;
    std::uint64_t observed_tick = 0;
    Clock::time_point prepared{}, started{}, deadline{}, last_service{};
    bool terminal() const {
        return phase == Phase::Stopped || phase == Phase::Refused || phase == Phase::SourceLost;
    }
};

class Engine {
public:
    Engine() = default;
    Engine(const Engine &) = delete;
    Engine &operator=(const Engine &) = delete;
    Engine(Engine &&) = delete;
    Engine &operator=(Engine &&) = delete;
    std::uint64_t sequence() const { return sequence_; }
    std::size_t size() const { return records_.size(); }
    bool active() const { return active_ != nullptr; }
    const Record *active_record() const { return active_; }
    template<class Reader> Snapshot observe(Reader read) const {
        auto value = read(); value.sequence = sequence_; return value;
    }
    const Record *query(const std::string &key, const std::string &plan) const {
        valid_key(key); require(plan.size() == 32);
        const auto it = records_.find(key);
        if (it == records_.end()) return nullptr;
        require(it->second.plan == plan, 7); return &it->second;
    }
    template<class Reader> const Record &prepare(const std::string &key, const std::string &plan,
        Spec spec, const Snapshot &expected, Clock::time_point now, Reader read) {
        valid_key(key); require(plan.size() == 32); spec.validate();
        if (const auto *old = query(key, plan)) {
            require(old->spec == spec && old->before == expected, 7);
            return *old; // Retry never renews the preparation lifetime.
        }
        require(!active_, 8);
        require(records_.size() < MAX_RECORDS && sequence_ != UINT64_MAX, 5);
        const auto current = observe(read);
        require(current.usable() && current.paused, 4);
        require(current == expected, 6);
        require(current.tick <= UINT64_MAX - spec.game_ticks, 5);
        require(now <= Clock::time_point::max() - PREPARE_LIFETIME, 5);
        Record value; value.key = key; value.plan = plan; value.spec = spec;
        value.before = current; value.prepared = now;
        return records_.emplace(key, std::move(value)).first->second;
    }
    template<class Reader, class Writer> const Record &commit(const std::string &key,
        const std::string &plan, Clock::time_point now, Reader read, Writer write) {
        const auto *known = query(key, plan); require(known, 7);
        auto &record = records_.find(key)->second;
        if (record.phase != Phase::Prepared) return record;
        require(!active_, 8);
        bool valid = sequence_ != UINT64_MAX && record.before.sequence == sequence_
            && now >= record.prepared && now < record.prepared + PREPARE_LIFETIME
            && now <= Clock::time_point::max() - std::chrono::milliseconds(record.spec.wall_ms);
        try { if (valid) valid = observe(read) == record.before; }
        catch (...) { valid = false; }
        if (!valid) {
            record.phase = Phase::Refused; record.reason = Reason::Stale; return record;
        }
        // Everything needed to retain clock ownership is published BEFORE the
        // setter. No allocation, receipt construction or reply delivery is needed
        // by service() to pause again after an ambiguous unpause.
        ++sequence_;
        record.started = now; record.last_service = now;
        record.deadline = now + std::chrono::milliseconds(record.spec.wall_ms);
        record.phase = Phase::Running; record.unpause_attempted = true;
        active_ = &record;
        try { write(record.before.generation, false); }
        catch (...) { begin_stop(record, Reason::NativeFailure); }
        service(now, read, write);
        return record;
    }
    template<class Reader, class Writer> void service(Clock::time_point now, Reader read, Writer write) {
        if (!active_) return;
        auto &record = *active_;
        if (now < record.last_service) begin_stop(record, Reason::ClockRegression);
        record.last_service = now;
        if (now >= record.deadline) begin_stop(record, Reason::WallLimit);
        try {
            const auto current = observe(read);
            if (current.generation != record.before.generation) {
                lose_source(record); return; // Never pause a replacement fortress.
            }
            if (!current.loaded || !current.clock_valid) {
                record.tick_known = false; begin_stop(record, Reason::NativeFailure);
            } else {
                if (current.tick < record.before.tick
                    || (record.tick_known && current.tick < record.observed_tick))
                    begin_stop(record, Reason::ClockRegression);
                record.observed_tick = current.tick; record.tick_known = true;
                if (current.tick >= record.before.tick + record.spec.game_ticks)
                    begin_stop(record, Reason::TickLimit);
                if (current.paused) {
                    if (record.phase == Phase::Running) record.reason = Reason::ExternalPause;
                    finish(record); return;
                }
            }
        } catch (...) {
            record.tick_known = false; begin_stop(record, Reason::NativeFailure);
        }
        if (record.phase == Phase::Stopping) attempt_pause(record, read, write);
    }
    template<class Reader, class Writer> const Record &cancel(const std::string &key,
        const std::string &plan, Clock::time_point now, Reader read, Writer write) {
        const auto *known = query(key, plan); require(known, 7);
        auto &record = records_.find(key)->second;
        if (record.phase == Phase::Prepared) {
            record.phase = Phase::Refused; record.reason = Reason::Cancelled;
        } else if (&record == active_) {
            begin_stop(record, Reason::Cancelled); service(now, read, write);
        }
        return record;
    }
    template<class Reader, class Writer> bool shutdown(Clock::time_point now, Reader read, Writer write) {
        if (active_) { begin_stop(*active_, Reason::Shutdown); service(now, read, write); }
        // The plugin must refuse unloading while a same-source pause is unverified.
        return !active_;
    }
    void source_changed() {
        if (sequence_ != UINT64_MAX) ++sequence_;
        if (active_) lose_source(*active_);
        for (auto &entry : records_) {
            auto &record = entry.second;
            if (record.phase == Phase::Prepared) {
                record.phase = Phase::Refused; record.reason = Reason::SourceChanged;
            }
        }
    }
private:
    static void begin_stop(Record &record, Reason reason) {
        if (record.phase == Phase::Running) { record.phase = Phase::Stopping; record.reason = reason; }
    }
    void finish(Record &record) {
        record.pause_verified = true; record.phase = Phase::Stopped; active_ = nullptr;
    }
    void lose_source(Record &record) {
        record.phase = Phase::SourceLost; record.reason = Reason::SourceChanged;
        record.pause_verified = false; record.tick_known = false; active_ = nullptr;
    }
    template<class Reader, class Writer> void attempt_pause(Record &record, Reader read, Writer write) {
        try {
            // Repeating a safety pause is intentional; repeating unpause is not.
            // This writer independently checks incarnation even if read() failed.
            write(record.before.generation, true);
        } catch (...) { /* Readback may still prove that an ambiguous setter paused. */ }
        try {
            const auto current = observe(read);
            if (current.generation != record.before.generation) { lose_source(record); return; }
            record.tick_known = current.loaded && current.clock_valid;
            if (record.tick_known) record.observed_tick = current.tick;
            if (current.loaded && current.paused) finish(record);
        } catch (...) { record.tick_known = false; }
        // Otherwise retain Stopping and ownership: no new run may start.
    }
    std::uint64_t sequence_ = 0;
    std::map<std::string, Record> records_;
    Record *active_ = nullptr;
};
} // namespace dfmcp_bounded_run
