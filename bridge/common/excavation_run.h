#pragma once
#include "bounded_run.h"
#include <array>
#include <optional>

// Excavation-conditioned clock ownership, not designation or mining causality.
// Beads: df-dfhack-bridge-plane-c-pic.4/.5, df-action-coordinator-exec-ero.4.
// Every entry and callback is serialized by the native owner. Clocks and all
// game effects are injected. No native pointer or background task is retained.
namespace dfmcp_excavation_run {
namespace clock = dfmcp_bounded_run;
using clock::Clock;
using clock::require;
constexpr std::uint64_t MAX_TICK = std::uint64_t{UINT32_MAX} * 403200 + 403199;
constexpr std::size_t MAX_CELLS = 64;

inline bool utf8(const std::string &v, std::size_t maximum) {
    if (v.empty() || v.size() > maximum) return false;
    for (std::size_t i = 0; i < v.size();) {
        const auto c = static_cast<unsigned char>(v[i]);
        if (!c) return false;
        if (c < 128) { ++i; continue; }
        const std::size_t n = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
        if (!n || i + n > v.size()) return false;
        for (std::size_t j = 1; j < n; ++j)
            if ((static_cast<unsigned char>(v[i + j]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(v[i + 1]);
        if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0)
            || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
struct Region {
    std::uint32_t x = 0, y = 0, z = 0, width = 0, height = 0;
    void validate() const {
        require(width >= 1 && width <= 8 && height >= 1 && height <= 8
            && x < 32768 && y < 32768 && z < 32768
            && width <= 32768 - x && height <= 32768 - y);
    }
    std::size_t size() const { validate(); return width * height; }
    bool operator==(const Region &b) const {
        return x == b.x && y == b.y && z == b.z && width == b.width && height == b.height;
    }
};
struct Identity {
    std::uint64_t generation = 0;
    std::uint32_t site = 0, map_x = 0, map_y = 0, map_z = 0;
    std::string folder;
    void validate() const {
        require(generation && generation != UINT64_MAX && site <= INT32_MAX
            && map_x >= 1 && map_x <= 32768 && map_y >= 1 && map_y <= 32768
            && map_z >= 1 && map_z <= 32768 && utf8(folder, 512), 5);
    }
    bool operator==(const Identity &b) const {
        return generation == b.generation && site == b.site && folder == b.folder
            && map_x == b.map_x && map_y == b.map_y && map_z == b.map_z;
    }
};
struct Source {
    clock::Snapshot clock;
    Identity identity;
    void validate() const {
        identity.validate();
        require(clock.generation == identity.generation && clock.loaded
            && (clock.clock_valid ? clock.tick <= MAX_TICK : clock.tick == 0), 5);
    }
    bool operator==(const Source &b) const { return clock == b.clock && identity == b.identity; }
};
struct Cell {
    // Missing/hidden cells carry no attributes. Shape tags follow map/1.5:
    // 0 unsupported, 1 empty, 2 wall, 3 floor, 4..8 ramps/stairs.
    std::uint8_t presence = 0, shape = 0, liquid = 0, dig = 0;
    void validate() const {
        require(presence <= 2 && shape <= 8 && liquid <= 7 && dig <= 7
            && (presence == 2 || (!shape && !liquid && !dig)), 5);
    }
    bool matches() const { return presence == 2 && shape == 3 && !liquid && !dig; }
    bool operator==(const Cell &b) const {
        return presence == b.presence && shape == b.shape && liquid == b.liquid && dig == b.dig;
    }
};
struct Capture {
    Source source;
    Region region;
    std::array<Cell, MAX_CELLS> cells{};
    void validate() const {
        source.validate(); region.validate();
        require(source.clock.usable() && region.x + region.width <= source.identity.map_x
            && region.y + region.height <= source.identity.map_y && region.z < source.identity.map_z, 5);
        const auto count = region.size();
        for (std::size_t i = 0; i < cells.size(); ++i) {
            cells[i].validate();
            require(i < count || cells[i] == Cell{}, 5);
        }
    }
    bool matches() const {
        for (std::size_t i = 0; i < region.size(); ++i) if (!cells[i].matches()) return false;
        return true;
    }
    bool operator==(const Capture &b) const {
        return source == b.source && region == b.region && cells == b.cells;
    }
};
struct Goal {
    std::uint32_t samples = 2, stable_ticks = 10, interval = 1, max_gap = 100;
    void validate(clock::Spec limits) const {
        limits.validate();
        require(samples >= 1 && samples <= 128 && interval >= 1 && interval <= limits.game_ticks
            && stable_ticks <= limits.game_ticks && max_gap >= interval && max_gap <= clock::MAX_GAME_TICKS);
        // Limits win at the boundary, so even the earliest eligible window must
        // fit strictly inside the run horizon. All arithmetic is bounded above.
        const auto span = (samples - 1) * interval;
        require(interval + (span > stable_ticks ? span : stable_ticks) < limits.game_ticks);
    }
    bool operator==(const Goal &b) const {
        return samples == b.samples && stable_ticks == b.stable_ticks && interval == b.interval && max_gap == b.max_gap;
    }
};
enum class Trigger : unsigned char {
    None = 0, FloorObserved = 1, SourceChanged = 2, CaptureFailure = 3,
    Unobservable = 4, LiquidObserved = 5
};
struct Record {
    // Points only into our own bounded, stable clock-record map.
    const clock::Record *run = nullptr;
    Capture before;
    Goal goal;
    Trigger trigger = Trigger::None;
    std::uint32_t stable_samples = 0;
    std::uint64_t first_stable_tick = 0, counted_tick = 0, last_tick = 0;
    std::optional<Capture> sample;
};

class Engine {
public:
    bool active() const { return clock_.active(); }
    std::size_t size() const { return records_.size(); }
    std::uint64_t sequence() const { return clock_.sequence(); }
    const Record *query(const std::string &key, const std::string &plan) const {
        const auto *value = clock_.query(key, plan);
        if (!value) return nullptr;
        const auto found = records_.find(key);
        require(found != records_.end() && found->second.run == value, 5);
        return &found->second;
    }
    template<class CaptureReader> Capture observe(Region region, CaptureReader read) const {
        region.validate();
        auto out = read(region); out.source.clock.sequence = sequence(); out.validate();
        require(out.region == region, 5); return out;
    }
    template<class CaptureReader> const Record &prepare(const std::string &key, const std::string &plan,
        clock::Spec limits, Goal goal, const Capture &before, Clock::time_point now, CaptureReader read) {
        clock::valid_key(key); require(plan.size() == 32);
        if (const auto *old = query(key, plan)) {
            require(old->run->spec == limits && old->goal == goal && old->before == before, 7);
            return *old;
        }
        require(!active(), 8); require(size() < clock::MAX_RECORDS, 5);
        before.validate(); goal.validate(limits);
        require(before.source.clock.paused && !before.matches(), 4);
        require(before.source.clock.tick <= MAX_TICK - limits.game_ticks, 5);
        for (std::size_t i = 0; i < before.region.size(); ++i)
            require(before.cells[i].presence == 2 && !before.cells[i].liquid, 4);
        Record record; record.before = before; record.goal = goal;
        record.counted_tick = record.last_tick = before.source.clock.tick;
        auto inserted = records_.emplace(key, std::move(record));
        try {
            inserted.first->second.run = &clock_.prepare(key, plan, limits, before.source.clock, now, [&] {
                const auto current = observe(before.region, read);
                require(current == before, 6); return current.source.clock;
            });
        } catch (...) { records_.erase(inserted.first); throw; }
        return inserted.first->second;
    }
    template<class SourceReader, class CaptureReader, class Writer> const Record &commit(
        const std::string &key, const std::string &plan, Clock::time_point now,
        SourceReader source, CaptureReader capture, Writer write) {
        require(query(key, plan), 7); auto &record = records_.find(key)->second;
        if (record.run->phase != clock::Phase::Prepared) return record;
        bool first = true;
        const auto read = [&] {
            if (first) {
                first = false;
                const auto current = observe(record.before.region, capture);
                require(current == record.before, 6);
            }
            return scoped_source(source(), record.before.source.identity);
        };
        clock_.commit(key, plan, now, read, [&](std::uint64_t g, bool p) {
            require(g == record.before.source.identity.generation, 4); write(record.before.source.identity, p);
        });
        return record; // The next native update owns sampling, even after disconnect.
    }
    template<class SourceReader, class CaptureReader, class Writer> void service(
        Clock::time_point now, SourceReader source, CaptureReader capture, Writer write) {
        const auto *run = clock_.active_record(); if (!run) return;
        auto &record = records_.find(run->key)->second;
        const auto read = [&] { return scoped_source(source(), record.before.source.identity); };
        const auto setter = [&](std::uint64_t g, bool p) {
            require(g == record.before.source.identity.generation, 4); write(record.before.source.identity, p);
        };
        // Clock checks never depend on a successful terrain read. Capture failure
        // must not prevent a safety pause or verification of that pause.
        clock_.service(now, read, setter);
        if (record.run->phase == clock::Phase::SourceLost) {
            if (record.trigger == Trigger::None) record.trigger = Trigger::SourceChanged;
            return;
        }
        if (record.run->phase != clock::Phase::Running) return;
        try {
            auto current = observe(record.before.region, capture);
            if (!(current.source.identity == record.before.source.identity)) { source_changed(); return; }
            const auto tick = current.source.clock.tick;
            // A second read may contradict the clock check. Defer to clock safety
            // before interpreting terrain, even within this serialized callback.
            if (tick < record.last_tick || tick < record.run->observed_tick
                || current.source.clock.paused || tick >= run->before.tick + run->spec.game_ticks) {
                clock_.service(now, [&] { return current.source.clock; }, setter); return;
            }
            Trigger trigger = Trigger::None;
            for (std::size_t i = 0; i < current.region.size(); ++i) {
                if (current.cells[i].presence != 2) trigger = Trigger::Unobservable;
                else if (current.cells[i].liquid && trigger == Trigger::None) trigger = Trigger::LiquidObserved;
            }
            if (tick - record.last_tick > record.goal.max_gap || !current.matches() || trigger != Trigger::None) {
                record.stable_samples = 0; record.first_stable_tick = 0;
            }
            if (trigger == Trigger::None && current.matches() && tick > record.last_tick
                && tick - record.counted_tick >= record.goal.interval) {
                if (!record.stable_samples) record.first_stable_tick = tick;
                if (record.stable_samples < record.goal.samples) ++record.stable_samples;
                record.counted_tick = tick;
                if (record.stable_samples >= record.goal.samples && tick - record.first_stable_tick >= record.goal.stable_ticks)
                    trigger = Trigger::FloorObserved;
            }
            record.last_tick = tick; record.sample = std::move(current);
            if (trigger != Trigger::None) {
                // Publish the entire witnessed trigger before the safety setter.
                record.trigger = trigger;
                clock_.cancel(run->key, run->plan, now, read, setter);
            }
        } catch (...) {
            record.stable_samples = 0; record.first_stable_tick = 0;
            if (record.trigger == Trigger::None) record.trigger = Trigger::CaptureFailure;
            clock_.cancel(run->key, run->plan, now, read, setter);
        }
    }
    template<class SourceReader, class Writer> const Record &cancel(const std::string &key,
        const std::string &plan, Clock::time_point now, SourceReader source, Writer write) {
        require(query(key, plan), 7); auto &record = records_.find(key)->second;
        clock_.cancel(key, plan, now,
            [&] { return scoped_source(source(), record.before.source.identity); },
            [&](std::uint64_t g, bool p) { require(g == record.before.source.identity.generation, 4); write(record.before.source.identity, p); });
        return record;
    }
    template<class SourceReader, class Writer> bool shutdown(Clock::time_point now, SourceReader source, Writer write) {
        const auto *run = clock_.active_record(); if (!run) return true;
        const auto &record = records_.find(run->key)->second;
        return clock_.shutdown(now,
            [&] { return scoped_source(source(), record.before.source.identity); },
            [&](std::uint64_t g, bool p) { require(g == record.before.source.identity.generation, 4); write(record.before.source.identity, p); });
    }
    void source_changed() {
        if (const auto *run = clock_.active_record()) {
            auto &record = records_.find(run->key)->second;
            if (record.trigger == Trigger::None) record.trigger = Trigger::SourceChanged;
        }
        clock_.source_changed();
    }
private:
    static clock::Snapshot scoped_source(Source source, const Identity &expected) {
        source.validate();
        if (!(source.identity == expected)) source.clock.generation = 0;
        return source.clock; // Internal source-loss sentinel; never serialized.
    }
    clock::Engine clock_;
    std::map<std::string, Record> records_;
};
} // namespace dfmcp_excavation_run
