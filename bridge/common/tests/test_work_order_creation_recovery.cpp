// Real production engine with deterministic native queue/configuration callbacks.
#include "../work_order_creation.h"
#include <functional>
#include <iostream>
#include <stdexcept>

namespace wo = dfmcp_work_orders;
static void check(bool ok, const char *why) { if (!ok) throw std::runtime_error(why); }

struct Fixture {
    wo::Engine engine{7};
    wo::Observation queue;
    wo::Spec spec;
    wo::Clock::time_point now{std::chrono::seconds(10)};
    std::string key = "create-recovery", plan, token, native_config;
    int reads = 0, writes = 0, verifies = 0;
    std::function<void()> on_read, on_write, on_verify;

    explicit Fixture(wo::Spec request = {wo::Recipe::WoodenBed, 1}) : spec(request) {
        queue.next_order = 3; queue.site = 1; queue.tick = 100;
        queue.paused = true; queue.folder = "region1"; queue.ids = {1, 2};
        const auto before = engine.inspect(reader());
        plan = wo::plan_digest(spec, before.witness());
        token = engine.prepare(key, spec, before.witness(), plan, now, reader()).first->token;
        reads = 0;
    }
    std::function<wo::Observation()> reader() {
        return [this] { ++reads; if (on_read) on_read(); return queue; };
    }
    void insert() {
        native_config = wo::configuration(queue.next_order, spec);
        queue.ids.push_back(queue.next_order++);
    }
    const wo::Record &commit() {
        return engine.commit(key, plan, token, now, reader(),
            [this](std::uint32_t id, wo::Spec request) {
                check(id == 3 && request.encode() == spec.encode(), "wrong creation request");
                ++writes; if (on_write) on_write();
            }, [this](std::uint32_t id, wo::Spec request) {
                check(id == 3 && request.encode() == spec.encode(), "wrong verification request");
                ++verifies; if (on_verify) on_verify(); return native_config;
            });
    }
    void verify(wo::State expected) {
        const auto &r = commit();
        check(r.state == expected, "wrong creation outcome");
        check(writes == 1, "insertion must be attempted exactly once");
        const bool created = expected == wo::State::Created;
        check(engine.unresolved() != created, "incorrect creation guard");
        check(r.after_known == created, "incorrect evidence visibility");
        if (created) {
            check(r.receipt == r.proof(), "invalid creation receipt");
            check(r.configuration_witness == dfmcp_snapshot::sha256(native_config), "wrong configuration proof");
            check(r.after_witness == engine.inspect([this] { return queue; }).witness(), "wrong queue proof");
        } else {
            check(r.receipt == std::string(32, '\0') && r.after_witness == std::string(32, '\0')
                && r.configuration_witness == std::string(32, '\0'), "partial proof leaked");
        }
        const auto bytes = r.encode();
        check(bytes.size() <= wo::MAX_EFFECT_BYTES, "wire effect limit exceeded");
        const auto saved_reads = reads, saved_verifies = verifies;
        check(engine.query(key, plan)->encode() == bytes, "query changed evidence");
        check(commit().encode() == bytes, "replay changed evidence");
        check(writes == 1 && reads == saved_reads && verifies == saved_verifies, "replay invoked native work");
    }
    void verify_guard() {
        const auto before = engine.inspect([this] { return queue; });
        const auto next_plan = wo::plan_digest(spec, before.witness());
        bool blocked = false;
        try { (void)engine.prepare("another-order", spec, before.witness(), next_plan, now,
            [this] { return queue; }); }
        catch (const wo::Failure &e) { check(e.code == 8, "wrong blocking reason"); blocked = true; }
        check(blocked == engine.unresolved(), "new creation authority disagrees with guard");
    }
};

static void fully_verified_insertions_recover() {
    for (auto recipe : {wo::Recipe::WoodenBed, wo::Recipe::WoodenDoor,
                        wo::Recipe::WoodenTable, wo::Recipe::WoodenChair}) {
        for (std::uint32_t amount : {1u, wo::MAX_AMOUNT}) {
            for (bool throws : {true, false}) {
                Fixture f({recipe, amount});
                f.on_write = [&] { f.insert(); if (throws) throw std::runtime_error("lost insertion acknowledgement"); };
                f.verify(wo::State::Created);
                check(f.reads == 3 && f.verifies == 1, "configuration must be bracketed by exact queue reads");
                check(f.engine.sequence() == 1, "creation sequence not consumed exactly once");
                f.verify_guard();
            }
        }
    }
}

static void uncertain_insertions_block_further_creation() {
    for (int fault = 0; fault != 10; ++fault) {
        for (bool throws : {false, true}) {
            Fixture f;
            f.on_write = [&] {
                // An unchanged queue, partial insertion, malformed queue or
                // mismatched configuration must never produce a negative receipt.
                if (fault != 0) f.insert();
                if (fault == 1) f.queue.ids.pop_back();
                if (fault == 2) --f.queue.next_order;
                if (fault == 3) f.native_config.clear();
                if (fault == 4) f.native_config = wo::configuration(3, {wo::Recipe::WoodenChair, 2});
                if (fault == 5) ++f.queue.tick;
                if (fault == 6) f.queue.folder = "region2";
                if (fault == 7) ++f.queue.site;
                if (fault == 8) f.queue.paused = false;
                if (fault == 9) f.engine.interrupt();
                if (throws) throw std::runtime_error("ambiguous insertion");
            };
            f.verify(wo::State::Unknown);
            // Restore valid readable state; it is too late to attribute an effect.
            f.queue.next_order = 4; f.queue.ids = {1, 2, 3};
            f.verify_guard();
        }
    }
}

static void readback_failures_retain_unknown() {
    for (int stage = 0; stage != 3; ++stage) {
        Fixture f;
        f.on_write = [&] { f.insert(); throw std::runtime_error("ambiguous insertion"); };
        f.on_read = [&] {
            if (f.writes && ((stage == 0 && f.verifies == 0) || (stage == 2 && f.verifies)))
                throw std::runtime_error("queue readback unavailable");
        };
        f.on_verify = [&] { if (stage == 1) throw std::runtime_error("configuration unavailable"); };
        f.verify(wo::State::Unknown); f.verify_guard();
    }
}

static void configuration_callback_cannot_escape_fence() {
    for (int fault = 0; fault != 4; ++fault) {
        for (bool throws : {false, true}) {
            Fixture f;
            f.on_write = [&] { f.insert(); if (throws) throw std::runtime_error("ambiguous insertion"); };
            f.on_verify = [&] {
                if (fault == 0) f.engine.interrupt();
                if (fault == 1) ++f.queue.tick;
                if (fault == 2) f.queue.ids.pop_back();
                if (fault == 3) f.queue.folder = "replacement-world";
            };
            f.verify(wo::State::Unknown); f.verify_guard();
        }
    }
}

static void preflight_failures_do_not_insert() {
    for (int fault = 0; fault != 4; ++fault) {
        Fixture f;
        if (fault == 0) f.now += wo::PREPARE_LIFETIME;
        if (fault == 1) f.engine.interrupt();
        if (fault == 2) ++f.queue.next_order;
        if (fault == 3) f.on_read = [] { throw std::runtime_error("preflight failed"); };
        const auto &r = f.commit();
        check(r.state == wo::State::Refused && f.writes == 0 && f.verifies == 0, "invalid preflight dispatched");
        check(!f.engine.unresolved() && r.receipt == r.proof(), "refusal corrupted guard or receipt");
    }
}

int main() {
    try {
        fully_verified_insertions_recover();
        uncertain_insertions_block_further_creation();
        readback_failures_retain_unknown();
        configuration_callback_cannot_escape_fence();
        preflight_failures_do_not_insert();
        std::cout << "work order creation recovery: 51 cases passed\n";
    } catch (const std::exception &e) {
        std::cerr << "work order creation recovery: " << e.what() << '\n';
        return 1;
    }
}
