// Exercises the production transaction engine, without a DFHack SDK or game.
#include "../job_suspension.h"
#include <functional>
#include <iostream>
#include <stdexcept>
#include <vector>

namespace js = dfmcp_job_suspension;

static void check(bool condition, const char *message) {
    if (!condition) throw std::runtime_error(message);
}

struct Fixture {
    js::Engine engine{1};
    js::Observation job;
    js::Clock::time_point now{std::chrono::seconds(10)};
    std::string key = "job-recovery", plan, token;
    int reads = 0, writes = 0;
    bool desired;
    std::function<void()> on_read, on_write;

    explicit Fixture(bool initially_suspended = false)
        : desired(!initially_suspended) {
        job.job = 3; job.next_job = 4; job.site = 1;
        job.type = 10; job.holder = 2; job.holder_type = 1;
        job.tick = 100; job.paused = true; job.suspended = initially_suspended;
        job.holder_complete = job.supported = job.production_holder = true;
        job.folder = "region1"; job.type_key = "ConstructDoor";
        const auto before = engine.inspect(job.job, reader());
        plan = js::plan_digest(job.job, desired, before.witness());
        token = engine.prepare(key, job.job, desired, before.witness(), plan,
            now, reader()).first->token;
        reads = 0;
    }
    std::function<js::Observation(std::uint32_t)> reader() {
        return [this](std::uint32_t id) {
            check(id == job.job, "wrong read target");
            ++reads;
            if (on_read) on_read();
            return job;
        };
    }
    const js::Record &commit() {
        return engine.commit(key, plan, token, now, reader(),
            [this](std::uint32_t id, bool value) {
                check(id == job.job && value == desired, "wrong write target");
                ++writes;
                if (on_write) on_write();
            });
    }
    void verify(js::State state) {
        const auto &record = commit();
        check(record.state == state, "wrong execution outcome");
        check(writes == 1, "setter must run exactly once");
        check(record.after_known == record.terminal(), "invalid readback visibility");
        if (record.terminal()) {
            check(record.receipt == record.proof(), "invalid terminal receipt");
            check(record.after_tick == job.tick, "wrong receipt clock");
            check(record.after_suspended == job.suspended, "wrong receipt flag");
            check(record.after_witness == engine.inspect(job.job,
                [this](std::uint32_t) { return job; }).witness(), "wrong after witness");
        } else {
            check(record.receipt == std::string(32, '\0'), "unknown has a receipt");
            check(record.after_witness == std::string(32, '\0'), "partial proof leaked");
        }
        const auto encoded = record.encode();
        const auto saved_reads = reads;
        check(engine.query(key, plan)->encode() == encoded, "query changed evidence");
        check(commit().encode() == encoded, "replay changed evidence");
        check(writes == 1 && reads == saved_reads, "replay dispatched native work");
    }
};

static void success_and_exception_outcomes() {
    for (bool initially_suspended : {false, true}) {
        // Returning normally does not establish the effect; the fresh flag does.
        for (bool changed : {false, true}) {
            for (bool throws : {false, true}) {
                Fixture f(initially_suspended);
                f.on_write = [&] {
                    if (changed) f.job.suspended = f.desired;
                    if (throws) throw std::runtime_error("native setter failure");
                };
                f.verify(changed ? js::State::Applied : js::State::NotApplied);
                check(f.reads == 2, "both preflight and post-write reads required");
                check(f.engine.sequence() == 1, "one dispatch must reserve one sequence");
            }
        }
    }
}

static void unavailable_readback_is_permanent_unknown() {
    for (bool throws : {false, true}) {
        Fixture f;
        f.on_write = [&] {
            f.job.suspended = f.desired;
            if (throws) throw std::runtime_error("write acknowledgement lost");
        };
        f.on_read = [&] {
            if (f.writes) throw std::runtime_error("job disappeared");
        };
        f.verify(js::State::Unknown);
        f.on_read = {}; // Later visibility must not authorize delayed attribution.
        check(f.commit().state == js::State::Unknown, "unknown was retroactively promoted");
        check(f.writes == 1 && f.reads == 2, "unknown replay retried native work");
    }
}

static void changed_context_is_not_effect_proof() {
    using Change = std::function<void(js::Observation &)>;
    const std::vector<Change> changes{
        [](auto &o) { ++o.tick; }, [](auto &o) { ++o.next_job; },
        [](auto &o) { ++o.site; }, [](auto &o) { ++o.type; },
        [](auto &o) { ++o.holder; }, [](auto &o) { ++o.holder_type; },
        [](auto &o) { o.worker = 5; }, [](auto &o) { ++o.x; },
        [](auto &o) { ++o.y; }, [](auto &o) { ++o.z; },
        [](auto &o) { ++o.timer; }, [](auto &o) { ++o.attachments; },
        [](auto &o) { ++o.filters; }, [](auto &o) { o.repeating = true; },
        [](auto &o) { o.paused = false; }, [](auto &o) { o.holder_complete = false; },
        [](auto &o) { o.supported = false; }, [](auto &o) { o.production_holder = false; },
        [](auto &o) { o.folder = "region2"; }, [](auto &o) { o.type_key = "OtherJob"; },
        [](auto &o) { o.reaction = "CHANGED"; }
    };
    for (const auto &change : changes) {
        for (bool throws : {false, true}) {
            Fixture f;
            f.on_write = [&] {
                f.job.suspended = f.desired; change(f.job);
                if (throws) throw std::runtime_error("ambiguous write");
            };
            f.verify(js::State::Unknown);
        }
    }
}

static void interrupted_dispatch_is_not_effect_proof() {
    for (bool during_read : {false, true}) {
        for (bool throws : {false, true}) {
            Fixture f;
            f.on_write = [&] {
                f.job.suspended = f.desired;
                if (!during_read) f.engine.interrupt();
                if (throws) throw std::runtime_error("interrupted write");
            };
            f.on_read = [&] {
                if (f.writes && during_read) f.engine.interrupt();
            };
            f.verify(js::State::Unknown);
            check(f.engine.sequence() == 2, "interruption was not retained");
        }
    }
}

static void stale_preparations_never_write() {
    for (int failure = 0; failure != 4; ++failure) {
        Fixture f;
        if (failure == 0) f.now += js::PREPARE_LIFETIME;
        if (failure == 1) f.engine.interrupt();
        if (failure == 2) f.job.worker = 5;
        if (failure == 3) f.on_read = [] { throw std::runtime_error("preflight read failed"); };
        const auto &r = f.commit();
        check(r.state == js::State::Refused && f.writes == 0, "stale prepare dispatched");
        check(r.receipt == r.proof(), "refusal receipt missing");
        const auto reads = f.reads;
        check(f.commit().state == js::State::Refused, "refusal replay changed state");
        check(f.writes == 0 && f.reads == reads, "refusal replay dispatched");
    }
}

int main() {
    try {
        success_and_exception_outcomes();
        unavailable_readback_is_permanent_unknown();
        changed_context_is_not_effect_proof();
        interrupted_dispatch_is_not_effect_proof();
        stale_preparations_never_write();
        std::cout << "job suspension recovery: 60 cases passed\n";
    } catch (const std::exception &e) {
        std::cerr << "job suspension recovery: " << e.what() << '\n';
        return 1;
    }
}
