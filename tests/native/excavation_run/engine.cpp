#include "bridge/common/excavation_run.h"
#include <functional>
#include <iostream>
#include <stdexcept>
#include <type_traits>

namespace er = dfmcp_excavation_run;
namespace br = dfmcp_bounded_run;
static unsigned assertions = 0, scenarios = 0;
#define CHECK(expr) do { ++assertions; if (!(expr)) throw std::runtime_error(#expr); } while (false)
static void refused(const std::function<void()> &fn, unsigned code = 0) {
    bool caught = false;
    try { fn(); } catch (const br::Failure &e) { caught = true; CHECK(!code || code == e.code); }
    CHECK(caught);
}
struct Harness {
    er::Engine engine;
    er::Source value{{41, 0, 100, true, true, true}, {41, 2, 64, 64, 8, "region1"}};
    er::Region region{15, 15, 2, 2, 2};
    std::array<er::Cell, er::MAX_CELLS> cells{};
    br::Spec limits{100, 1000};
    er::Goal goal{2, 3, 1, 20};
    er::Clock::time_point now{};
    std::string plan = std::string(32, 'p');
    bool broken_capture = false, broken_source = false, fail_pause = false, throw_unpause = false;
    unsigned pauses = 0, unpauses = 0, reads = 0;
    Harness() { for (std::size_t i = 0; i < region.size(); ++i) cells[i] = {2, 2, 0, 1}; }
    auto source() { return [this] {
        if (broken_source) throw br::Failure(5);
        return value;
    }; }
    auto capture() { return [this](er::Region selected) {
        ++reads; if (broken_capture) throw br::Failure(5);
        return er::Capture{value, selected, cells};
    }; }
    auto writer() { return [this](const er::Identity &id, bool paused) {
        br::require(id == value.identity, 4);
        if (paused) { ++pauses; if (fail_pause) throw br::Failure(5); }
        else { ++unpauses; if (throw_unpause) { value.clock.paused = false; throw br::Failure(5); } }
        value.clock.paused = paused;
    }; }
    er::Capture observe() { return engine.observe(region, capture()); }
    const er::Record &prepare(const std::string &key = "mine") {
        return engine.prepare(key, plan, limits, goal, observe(), now, capture());
    }
    const er::Record &commit(const std::string &key = "mine") {
        return engine.commit(key, plan, now, source(), capture(), writer());
    }
    const er::Record &run() { prepare(); return commit(); }
    const er::Record &record() { return *engine.query("mine", plan); }
    void step(std::uint64_t tick, int ms = 1) {
        value.clock.tick = tick; now += std::chrono::milliseconds(ms);
        engine.service(now, source(), capture(), writer());
    }
    void floors() { for (std::size_t i = 0; i < region.size(); ++i) cells[i] = {2, 3, 0, 0}; }
};
static void predicate_matrix() {
    ++scenarios;
    for (unsigned shape = 0; shape <= 8; ++shape)
    for (unsigned liquid = 0; liquid <= 7; ++liquid)
    for (unsigned dig = 0; dig <= 7; ++dig) {
        er::Cell cell{2, static_cast<std::uint8_t>(shape), static_cast<std::uint8_t>(liquid), static_cast<std::uint8_t>(dig)};
        cell.validate(); CHECK(cell.matches() == (shape == 3 && liquid == 0 && dig == 0));
    }
    for (unsigned p = 0; p < 2; ++p) {
        er::Cell hidden{static_cast<std::uint8_t>(p), 0, 0, 0}; hidden.validate(); CHECK(!hidden.matches());
        hidden.shape = 3; refused([&] { hidden.validate(); });
        hidden.shape = 0; hidden.liquid = 1; refused([&] { hidden.validate(); });
        hidden.liquid = 0; hidden.dig = 1; refused([&] { hidden.validate(); });
    }
}
static void stable_goal_and_replays() {
    ++scenarios;
    Harness h; h.run(); CHECK(h.engine.active()); CHECK(h.unpauses == 1);
    h.floors(); h.step(101); CHECK(h.record().stable_samples == 1);
    for (int i = 0; i < 100; ++i) { h.step(101); CHECK(h.record().stable_samples == 1); }
    h.step(102); CHECK(h.engine.active()); CHECK(h.record().stable_samples == 2);
    h.step(104); CHECK(!h.engine.active()); CHECK(h.value.clock.paused);
    CHECK(h.record().trigger == er::Trigger::FloorObserved);
    CHECK(h.record().run->pause_verified); CHECK(h.record().sample->source.clock.tick == 104);
    CHECK(h.record().first_stable_tick == 101);
    const auto reads = h.reads, pauses = h.pauses;
    for (int i = 0; i < 10; ++i) {
        h.commit(); h.engine.cancel("mine", h.plan, h.now, h.source(), h.writer()); h.step(110 + i);
        CHECK(h.record().sample->source.clock.tick == 104);
    }
    CHECK(h.reads == reads); CHECK(h.pauses == pauses); CHECK(h.unpauses == 1);
}
static void contradictory_samples_reset() {
    ++scenarios;
    Harness h; h.goal = {2, 0, 3, 20}; h.run(); h.floors();
    h.step(103); CHECK(h.record().stable_samples == 1);
    h.cells[0].dig = 1; h.step(104); CHECK(h.record().stable_samples == 0);
    h.floors(); h.step(105); CHECK(h.record().stable_samples == 0);
    h.step(106); CHECK(h.record().stable_samples == 1);
    h.cells[0].shape = 2; h.step(106); CHECK(h.record().stable_samples == 0);
    h.floors(); h.step(106); CHECK(h.record().stable_samples == 0);
    h.step(109); CHECK(h.record().stable_samples == 1);
    h.step(112); CHECK(h.record().trigger == er::Trigger::FloorObserved);
    Harness same; same.goal = {2, 0, 3, 20}; same.run(); same.floors(); same.step(103);
    same.cells[0].shape = 2; same.step(106); CHECK(same.record().stable_samples == 0);
    same.floors(); same.step(106); CHECK(same.record().stable_samples == 0);
}
static void sample_gaps_reset() {
    ++scenarios;
    Harness h; h.goal = {2, 0, 1, 3}; h.run(); h.floors();
    h.step(101); h.step(106); CHECK(h.engine.active());
    CHECK(h.record().stable_samples == 1); CHECK(h.record().first_stable_tick == 106);
    h.step(107); CHECK(h.record().trigger == er::Trigger::FloorObserved);
}
static void visibility_and_water_stop() {
    ++scenarios;
    for (int mode = 0; mode < 3; ++mode) {
        Harness h; h.run(); h.floors(); h.step(101);
        h.cells[0] = mode == 2 ? er::Cell{2, 3, 1, 0} : er::Cell{static_cast<std::uint8_t>(mode), 0, 0, 0};
        h.step(102); CHECK(!h.engine.active()); CHECK(h.record().stable_samples == 0);
        CHECK(h.record().trigger == (mode == 2 ? er::Trigger::LiquidObserved : er::Trigger::Unobservable));
        CHECK(h.record().run->pause_verified); CHECK(h.record().sample.has_value());
    }
}
static void capture_failure_does_not_disable_pause() {
    ++scenarios;
    Harness h; h.run(); h.broken_capture = true; h.step(101);
    CHECK(!h.engine.active()); CHECK(h.value.clock.paused); CHECK(h.record().run->pause_verified);
    CHECK(h.record().trigger == er::Trigger::CaptureFailure); CHECK(!h.record().sample);
}
static void failed_pause_keeps_owner_and_trigger() {
    ++scenarios;
    Harness h; h.goal = {1, 0, 1, 20}; h.run(); h.floors(); h.fail_pause = true;
    h.step(101); CHECK(h.engine.active()); CHECK(h.record().run->phase == br::Phase::Stopping);
    CHECK(h.record().trigger == er::Trigger::FloorObserved); CHECK(!h.record().run->pause_verified);
    refused([&] { h.prepare("other"); }, 8);
    h.step(102); CHECK(h.engine.active()); CHECK(h.record().sample->source.clock.tick == 101);
    h.fail_pause = false; h.step(103); CHECK(!h.engine.active()); CHECK(h.record().run->pause_verified);
    CHECK(h.unpauses == 1); CHECK(h.pauses == 3);
}
static void source_substitution_never_pauses_replacement() {
    ++scenarios;
    for (int field = 0; field < 6; ++field) {
        Harness h; h.run();
        switch (field) {
            case 0: ++h.value.identity.generation; ++h.value.clock.generation; break;
            case 1: ++h.value.identity.site; break;
            case 2: h.value.identity.folder = "other"; break;
            case 3: ++h.value.identity.map_x; break;
            case 4: ++h.value.identity.map_y; break;
            default: ++h.value.identity.map_z; break;
        }
        h.step(101); CHECK(h.pauses == 0); CHECK(!h.engine.active());
        CHECK(h.record().run->phase == br::Phase::SourceLost); CHECK(!h.record().run->pause_verified);
    }
}
static void stale_before_and_lifetime() {
    ++scenarios;
    for (int field = 0; field < 6; ++field) {
        Harness h; h.prepare();
        switch (field) {
            case 0: ++h.value.clock.tick; break;
            case 1: h.cells[0].dig = 0; break;
            case 2: h.cells[0].shape = 3; break;
            case 3: h.now += br::PREPARE_LIFETIME; break;
            case 4: h.value.clock.paused = false; break;
            default: h.broken_capture = true; break;
        }
        h.commit(); CHECK(h.unpauses == 0); CHECK(h.record().run->phase == br::Phase::Refused);
        h.commit(); CHECK(h.unpauses == 0);
    }
    Harness h; const auto before = h.observe(); h.prepare();
    h.now += std::chrono::seconds(59);
    h.engine.prepare("mine", h.plan, h.limits, h.goal, before, h.now, h.capture());
    h.now += std::chrono::seconds(1); h.commit(); CHECK(h.unpauses == 0);
}
static void limits_and_pause_precede_goal() {
    ++scenarios;
    for (int mode = 0; mode < 4; ++mode) {
        Harness h; h.goal = {1, 0, 1, 20}; h.run(); h.floors();
        if (mode == 0) h.step(200);
        if (mode == 1) h.step(101, 1000);
        if (mode == 2) { h.value.clock.paused = true; h.step(101); }
        if (mode == 3) h.step(99);
        CHECK(!h.engine.active()); CHECK(h.record().trigger != er::Trigger::FloorObserved);
        CHECK(h.record().run->pause_verified);
        const br::Reason reasons[] = {br::Reason::TickLimit, br::Reason::WallLimit, br::Reason::ExternalPause, br::Reason::ClockRegression};
        CHECK(h.record().run->reason == reasons[mode]);
    }
}
static void cancellation_and_shutdown() {
    ++scenarios;
    Harness h; h.prepare(); h.engine.cancel("mine", h.plan, h.now, h.source(), h.writer());
    h.commit(); CHECK(!h.unpauses); CHECK(h.record().run->phase == br::Phase::Refused);
    Harness run; run.run(); run.fail_pause = true;
    CHECK(!run.engine.shutdown(run.now, run.source(), run.writer())); CHECK(run.engine.active());
    run.fail_pause = false; CHECK(run.engine.shutdown(run.now, run.source(), run.writer()));
    CHECK(run.record().run->reason == br::Reason::Shutdown); CHECK(run.unpauses == 1);
    CHECK(run.record().trigger == er::Trigger::None);
}
static void ambiguous_unpause_and_invalid_clock() {
    ++scenarios;
    Harness h; h.throw_unpause = true; h.run(); CHECK(h.unpauses == 1); CHECK(h.pauses == 1);
    CHECK(h.record().run->phase == br::Phase::Stopped); h.commit(); CHECK(h.unpauses == 1);
    Harness bad; bad.run(); bad.value.clock.clock_valid = false; bad.value.clock.tick = 0;
    bad.engine.service(bad.now, bad.source(), bad.capture(), bad.writer());
    CHECK(!bad.engine.active()); CHECK(bad.record().run->pause_verified); CHECK(!bad.record().run->tick_known);
}
static void source_events_retire_preparations() {
    ++scenarios;
    Harness h; h.prepare(); h.engine.source_changed(); h.commit(); CHECK(h.unpauses == 0);
    Harness run; run.run(); run.engine.source_changed(); CHECK(!run.engine.active()); CHECK(run.pauses == 0);
    CHECK(run.record().trigger == er::Trigger::SourceChanged); CHECK(run.record().run->phase == br::Phase::SourceLost);
}
static void already_true_unknown_and_bounds_refused() {
    ++scenarios;
    Harness true_goal; true_goal.floors(); refused([&] { true_goal.prepare(); }, 4); CHECK(!true_goal.unpauses);
    for (int mode = 0; mode < 3; ++mode) {
        Harness h; h.cells[0] = mode == 2 ? er::Cell{2, 2, 1, 0} : er::Cell{static_cast<std::uint8_t>(mode), 0, 0, 0};
        refused([&] { h.prepare(); }, 4); CHECK(h.engine.size() == 0);
    }
    for (er::Goal goal : {er::Goal{0,0,1,1}, {129,0,1,1}, {1,0,0,1}, {1,100,1,1}, {2,0,3,2}, {100,0,1,1}}) {
        Harness h; h.goal = goal; refused([&] { h.prepare(); }); CHECK(h.engine.size() == 0);
    }
    Harness h; h.region.width = 9; refused([&] { h.observe(); });
    h.region.width = 8; h.region.x = 32767; refused([&] { h.observe(); });
    h.region.x = 60; refused([&] { h.observe(); });
    Harness late; late.value.clock.tick = er::MAX_TICK - 99; refused([&] { late.prepare(); });
}
static void conflict_and_capacity() {
    ++scenarios;
    Harness h; const auto before = h.observe(); h.prepare();
    refused([&] { h.engine.query("mine", std::string(32, 'q')); }, 7);
    auto goal = h.goal; ++goal.stable_ticks;
    refused([&] { h.engine.prepare("mine", h.plan, h.limits, goal, before, h.now, h.capture()); }, 7);
    for (unsigned i = 1; i < br::MAX_RECORDS; ++i) h.prepare("other" + std::to_string(i));
    CHECK(h.engine.size() == br::MAX_RECORDS); refused([&] { h.prepare("overflow"); }, 5);
    h.commit(); CHECK(h.unpauses == 1);
    h.engine.cancel("mine", h.plan, h.now, h.source(), h.writer());
    h.commit("other1"); CHECK(h.unpauses == 1); // Dispatch sequence fences competitors.
}
static void bounded_sample_window_exhaustive() {
    ++scenarios;
    for (unsigned mask = 0; mask < 256; ++mask) {
        Harness h; h.goal = {3, 2, 1, 20}; h.run();
        unsigned streak = 0; bool expected = false;
        for (unsigned i = 0; i < 8; ++i) {
            const bool matching = (mask & (1u << i)) != 0;
            if (matching) h.floors(); else h.cells[0] = {2, 2, 0, 1};
            if (!expected) { streak = matching ? streak + 1 : 0; expected = streak >= 3; }
            h.step(101 + i);
            CHECK((h.record().trigger == er::Trigger::FloorObserved) == expected);
            CHECK(h.engine.active() != expected); CHECK(h.unpauses == 1);
        }
    }
}
int main() {
    static_assert(!std::is_copy_constructible_v<er::Engine>);
    try {
        predicate_matrix(); stable_goal_and_replays(); contradictory_samples_reset(); sample_gaps_reset();
        visibility_and_water_stop(); capture_failure_does_not_disable_pause(); failed_pause_keeps_owner_and_trigger();
        source_substitution_never_pauses_replacement(); stale_before_and_lifetime(); limits_and_pause_precede_goal();
        cancellation_and_shutdown(); ambiguous_unpause_and_invalid_clock(); source_events_retire_preparations();
        already_true_unknown_and_bounds_refused(); conflict_and_capacity(); bounded_sample_window_exhaustive();
        std::cout << "{\"scenarios\":" << scenarios << ",\"assertions\":" << assertions << "}\n";
    } catch (const std::exception &e) { std::cerr << e.what() << '\n'; return 1; }
}
