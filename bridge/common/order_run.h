#pragma once
#include "bounded_run.h"
#include <optional>

// Native order-run/1.14. An observed order predicate is a reason to stop the
// clock, NOT evidence of produced goods. All callbacks run under one suspension.
// Writer independently verifies generation AND folder/site before every setter.
namespace dfmcp_order_run {
namespace clock = dfmcp_bounded_run;
using clock::require;
using clock::Clock;
constexpr std::uint64_t MAX_TICK = std::uint64_t{UINT32_MAX} * 403200 + 403199;

struct Identity {
    std::uint64_t generation = 0;
    std::uint32_t site = 0;
    std::string folder;
    bool operator==(const Identity &b) const {
        return generation == b.generation && site == b.site && folder == b.folder;
    }
    void validate() const {
        require(generation && generation != UINT64_MAX && site <= INT32_MAX
            && !folder.empty() && folder.size() <= 512 && folder.find('\0') == std::string::npos, 5);
    }
};
struct Capture {
    clock::Snapshot clock;
    Identity identity;
    std::uint32_t id = 0, next_order = 0;
    bool present = false;
    // Only 1..4 are recognized static finite-wood templates; 0 is unrecognized.
    std::uint8_t recipe = 0;
    std::int32_t total = 0, left = 0;
    std::uint32_t status = 0;
    void validate() const {
        identity.validate();
        require(clock.usable() && clock.generation == identity.generation && clock.tick <= MAX_TICK
            && id <= INT32_MAX && next_order <= INT32_MAX && recipe <= 4, 5);
        if (!present) {
            require(recipe == 0 && total == 0 && left == 0 && status == 0, 5);
        } else {
            require(id < next_order && total >= INT16_MIN && total <= INT16_MAX
                && left >= INT16_MIN && left <= INT16_MAX, 5);
            require(recipe == 0 || (total >= 1 && total <= 100 && left >= 0
                && left <= total && (status & ~3u) == 0), 5);
        }
    }
    bool operator==(const Capture &b) const {
        return clock == b.clock && identity == b.identity && id == b.id && next_order == b.next_order
            && present == b.present && recipe == b.recipe && total == b.total && left == b.left && status == b.status;
    }
};
enum class Predicate : unsigned char { Approved = 1, Active = 2, RemainingAtMost = 3 };
struct Goal {
    Predicate predicate = Predicate::Approved;
    std::uint32_t threshold = 0, samples = 1, interval = 1;
    void validate(clock::Spec limits, const Capture &before) const {
        limits.validate(); before.validate();
        const auto code = static_cast<unsigned>(predicate);
        require(code >= 1 && code <= 3 && samples >= 1 && samples <= 16
            && interval >= 1 && interval <= clock::MAX_GAME_TICKS
            && samples * interval <= limits.game_ticks);
        require(before.clock.paused && before.present && before.recipe != 0, 4);
        require(threshold <= static_cast<std::uint32_t>(before.total)
            && (predicate == Predicate::RemainingAtMost || threshold == 0));
    }
    bool matches(const Capture &value) const {
        if (!value.present || value.recipe == 0) return false;
        switch (predicate) {
            case Predicate::Approved: return (value.status & 1u) != 0;
            case Predicate::Active: return (value.status & 2u) != 0;
            case Predicate::RemainingAtMost: return value.left <= static_cast<std::int32_t>(threshold);
        }
        return false;
    }
    bool operator==(const Goal &b) const {
        return predicate == b.predicate && threshold == b.threshold && samples == b.samples && interval == b.interval;
    }
};
enum class Trigger : unsigned char {
    None = 0, PredicateObserved = 1, TargetAbsent = 2, TargetChanged = 3,
    CounterRegression = 4, HorizonRegression = 5, SourceChanged = 6, NativeFailure = 7
};
struct Record {
    // Pointer is to this engine's stable map node, never to game memory.
    const clock::Record *run = nullptr;
    Capture before;
    Goal goal;
    Trigger trigger = Trigger::None;
    std::uint32_t stable_samples = 0;
    std::uint64_t counted_tick = 0, last_tick = 0;
    std::int32_t last_left = 0;
    std::uint32_t last_horizon = 0;
    std::optional<Capture> sample;
    bool predicate_observed() const { return trigger == Trigger::PredicateObserved; }
};

class Engine {
public:
    bool active() const { return clock_.active(); }
    std::size_t size() const { return records_.size(); }
    std::uint64_t sequence() const { return clock_.sequence(); }
    const Record *query(const std::string &key, const std::string &plan) const {
        const auto *run = clock_.query(key, plan);
        if (!run) return nullptr;
        const auto found = records_.find(key);
        require(found != records_.end() && found->second.run == run, 5);
        return &found->second;
    }
    template<class Reader> Capture observe(std::uint32_t id, Reader read) const {
        require(id <= INT32_MAX);
        auto out = read(id); out.clock.sequence = sequence(); out.validate();
        require(out.id == id, 5); return out;
    }
    template<class Reader> const Record &prepare(const std::string &key, const std::string &plan,
        clock::Spec limits, Goal goal, const Capture &before, Clock::time_point now, Reader read) {
        clock::valid_key(key); require(plan.size() == 32);
        if (const auto *old = query(key, plan)) {
            require(old->run->spec == limits && old->goal == goal && old->before == before, 7);
            return *old;
        }
        require(!active(), 8); require(size() < clock::MAX_RECORDS, 5);
        goal.validate(limits, before);
        require(!goal.matches(before), 4); // Already true: never unpause just to observe it again.
        Record value; value.before = before; value.goal = goal; value.counted_tick = before.clock.tick;
        value.last_tick = before.clock.tick; value.last_left = before.left; value.last_horizon = before.next_order;
        auto result = records_.emplace(key, std::move(value));
        try {
            result.first->second.run = &clock_.prepare(key, plan, limits, before.clock, now, [&] {
                auto current = observe(before.id, read); require(current == before, 6); return current.clock;
            });
        } catch (...) { records_.erase(result.first); throw; }
        return result.first->second;
    }
    template<class Reader, class Writer> const Record &commit(const std::string &key,
        const std::string &plan, Clock::time_point now, Reader read, Writer write) {
        require(query(key, plan), 7); auto &record = records_.find(key)->second;
        if (record.run->phase != clock::Phase::Prepared) return record;
        bool first = true;
        const auto scoped_read = [&] {
            auto current = observe(record.before.id, read);
            if (first) { first = false; require(current == record.before, 6); }
            return scoped_clock(current, record.before.identity);
        };
        clock_.commit(key, plan, now, scoped_read, [&](std::uint64_t generation, bool paused) {
            require(generation == record.before.identity.generation, 4);
            write(record.before.identity, paused);
        });
        service(now, read, write);
        return record;
    }
    template<class Reader, class Writer> void service(Clock::time_point now, Reader read, Writer write) {
        const auto *active_run = clock_.active_record();
        if (!active_run) return;
        auto &record = records_.find(active_run->key)->second;
        const auto scoped_read = [&] {
            return scoped_clock(observe(record.before.id, read), record.before.identity);
        };
        const auto scoped_write = [&](std::uint64_t generation, bool paused) {
            require(generation == record.before.identity.generation, 4); write(record.before.identity, paused);
        };
        // Clock failures, existing stopping, wall/tick limits and external pauses
        // take precedence over a new predicate claim. Readback is always fresh.
        clock_.service(now, scoped_read, scoped_write);
        if (record.run->phase == clock::Phase::SourceLost) {
            if (record.trigger == Trigger::None) record.trigger = Trigger::SourceChanged;
            return;
        }
        if (!active() || record.run->phase != clock::Phase::Running) return;
        try {
            auto current = observe(record.before.id, read);
            if (!(current.identity == record.before.identity)) { source_changed(); return; }
            // A regression must be handled by the clock safety machinery, never
            // counted as a new positive sample in this second capture.
            if (current.clock.tick < record.last_tick || current.clock.tick < record.run->observed_tick) {
                clock_.service(now, [&] { return current.clock; }, scoped_write); return;
            }
            if (current.clock.paused || current.clock.tick >= record.before.clock.tick + record.run->spec.game_ticks) {
                clock_.service(now, [&] { return current.clock; }, scoped_write); return;
            }
            Trigger trigger = Trigger::None;
            if (current.next_order < record.last_horizon) trigger = Trigger::HorizonRegression;
            else if (!current.present) trigger = Trigger::TargetAbsent;
            else if (current.recipe == 0 || current.recipe != record.before.recipe || current.total != record.before.total)
                trigger = Trigger::TargetChanged;
            else if (current.left > record.last_left) trigger = Trigger::CounterRegression;
            else if (!record.goal.matches(current)) record.stable_samples = 0;
            else if (current.clock.tick >= record.counted_tick + record.goal.interval) {
                ++record.stable_samples; record.counted_tick = current.clock.tick;
                if (record.stable_samples == record.goal.samples) trigger = Trigger::PredicateObserved;
            }
            record.last_tick = current.clock.tick; record.last_left = current.left; record.last_horizon = current.next_order;
            // Move the complete captured value, then publish the trigger before
            // the safety setter. Reply allocation can fail without losing either.
            record.sample = std::move(current);
            if (trigger != Trigger::None) {
                record.trigger = trigger;
                clock_.cancel(active_run->key, active_run->plan, now, scoped_read, scoped_write);
            }
        } catch (...) {
            if (record.trigger == Trigger::None) record.trigger = Trigger::NativeFailure;
            clock_.service(now, []() -> clock::Snapshot { throw clock::Failure(5); }, scoped_write);
        }
    }
    template<class Reader, class Writer> const Record &cancel(const std::string &key,
        const std::string &plan, Clock::time_point now, Reader read, Writer write) {
        require(query(key, plan), 7); auto &record = records_.find(key)->second;
        clock_.cancel(key, plan, now,
            [&] { return scoped_clock(observe(record.before.id, read), record.before.identity); },
            [&](std::uint64_t g, bool p) { require(g == record.before.identity.generation, 4); write(record.before.identity, p); });
        return record;
    }
    template<class Reader, class Writer> bool shutdown(Clock::time_point now, Reader read, Writer write) {
        const auto *run = clock_.active_record(); if (!run) return true;
        const auto &record = records_.find(run->key)->second;
        return clock_.shutdown(now,
            [&] { return scoped_clock(observe(record.before.id, read), record.before.identity); },
            [&](std::uint64_t g, bool p) { require(g == record.before.identity.generation, 4); write(record.before.identity, p); });
    }
    void source_changed() {
        if (const auto *run = clock_.active_record()) {
            auto &record = records_.find(run->key)->second;
            if (record.trigger == Trigger::None) record.trigger = Trigger::SourceChanged;
        }
        clock_.source_changed();
    }
private:
    static clock::Snapshot scoped_clock(const Capture &value, const Identity &expected) {
        auto out = value.clock;
        // Internal discontinuity sentinel. The base engine sees the mismatch,
        // marks SourceLost, and performs no setter. Never serialized as a read.
        if (!(value.identity == expected)) out.generation = 0;
        return out;
    }
    clock::Engine clock_;
    std::map<std::string, Record> records_;
};
} // namespace dfmcp_order_run
