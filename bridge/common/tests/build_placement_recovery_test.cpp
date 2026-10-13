// Bead: df-dfhack-bridge-plane-c-pic.4. Actual value engine, injected native boundaries.
#include "../build_placement.h"
#include <functional>
#include <iostream>
#include <stdexcept>
using namespace dfmcp_build;
namespace {
unsigned checks = 0, scenarios = 0;
#define CHECK(x) do { ++checks; if (!(x)) throw std::runtime_error("check failed: " #x); } while (false)
void refused(const std::function<void()> &f, unsigned code) {
    bool caught = false;
    try { f(); } catch (const Failure &e) { caught = true; CHECK(e.code == code); }
    CHECK(caught);
}
Capture initial(Kind kind = Kind::Bed) {
    Capture c; c.generation = 7; c.tick = 12345; c.site = 3; c.folder = "fortress";
    c.dimensions = {64,64,8}; c.next_building = 20; c.next_job = 30; c.building_count = 4;
    c.paused = c.free_tile = c.supported = true; c.selection = {kind,42,{15,15,2}};
    for (auto &t : c.tiles) { t.presence = Presence::Visible; t.tiletype = 2; t.shape = 3; }
    c.item.presence = Presence::Visible; c.item.kind = kind; c.item.pos = {3,3,2};
    c.item.native_type = 10; c.item.subtype = -1; c.item.material = 419; c.item.material_index = 5;
    c.item.on_ground = true; c.item.ground = c.tiles[0]; return c;
}
Insertion insertion(const Capture &c) {
    Insertion p; p.building = c.next_building; p.job = c.next_job; p.item = c.selection.item;
    p.kind = c.selection.kind; p.pos = c.selection.target; p.material = c.item.material;
    p.material_index = c.item.material_index; p.max_stage = 3;
    p.linked = p.construct_job = p.exact_item_link = true; return p;
}
struct Fixture {
    Engine engine{7}; Capture current, before; std::string plan, token;
    unsigned writers = 0, reads = 0, verifications = 0;
    auto reader() { return [this](const Selection &) { ++reads; return current; }; }
    auto verifier() { return [this](const Capture &c) { ++verifications; return insertion(c); }; }
    explicit Fixture(Kind kind = Kind::Bed) : current(initial(kind)), before(current) {
        plan = plan_digest(before.selection,before.witness()); token = token_for("original",plan);
        engine.prepare("original",before.selection,before.witness(),plan,100,reader());
    }
    const Record &record() const { const auto *r = engine.query("original",plan); CHECK(r); return *r; }
    // fault 0: first capture; 1: verifier; 2: final capture; 3: no effect;
    // 4: writer throws after effect plus read failure; 5: ordinary successful commit.
    void attempt(unsigned fault = 0) {
        unsigned post_reads = 0;
        engine.commit("original",plan,token,101,[&](const Selection &) {
            ++reads;
            if (writers) {
                ++post_reads;
                if (fault == 0 || fault == 4 || (fault == 2 && post_reads == 2)) throw std::runtime_error("capture");
            }
            return current;
        },[&](const Capture &c) {
            ++writers;
            if (fault != 3) current = c.expected_after();
            if (fault == 4) throw std::runtime_error("ambiguous setter");
        },[&](const Capture &c) {
            ++verifications; if (fault == 1) throw std::runtime_error("verify"); return insertion(c);
        });
        CHECK(writers == 1);
        CHECK(record().phase == (fault == 5 ? Phase::Placed : Phase::Indeterminate));
    }
    const Record *recover() { return engine.reconcile("original",plan,reader(),verifier()); }
    void unchanged(const std::string &bytes) const {
        CHECK(record().encode() == bytes); CHECK(record().phase == Phase::Indeterminate);
        CHECK(engine.unresolved()); CHECK(!record().after && !record().insertion); CHECK(writers == 1);
    }
};
void positive_recovery_and_byte_identity() {
    for (auto kind : {Kind::Bed,Kind::Chair,Kind::Table}) {
        Fixture direct(kind); direct.attempt(5); const auto expected = direct.record().encode();
        for (unsigned fault : {0u,1u,2u,4u}) {
            Fixture f(kind); f.attempt(fault); CHECK(f.engine.unresolved());
            const auto *address = &f.record(); const auto reads = f.reads, verifies = f.verifications;
            CHECK(f.recover() == address); CHECK(f.record().phase == Phase::Placed);
            CHECK(f.record().encode() == expected); CHECK(!f.engine.unresolved());
            CHECK(f.reads == reads + 2 && f.verifications == verifies + 1);
            CHECK(f.engine.sequence() == 1 && f.engine.size() == 1 && f.writers == 1);
            CHECK(f.record().created_ms == 100); // Recover, not a renewed prepare.
            const auto n = f.reads;
            f.current = initial(); f.engine.change_source();
            CHECK(f.recover()->encode() == expected); CHECK(f.reads == n && f.writers == 1);
        }
    }
}
void old_query_dead_end_and_fence_release() {
    Fixture f; f.attempt(); const auto bytes = f.record().encode(); const auto n = f.reads;
    CHECK(f.engine.query("original",f.plan)->encode() == bytes); CHECK(f.reads == n);
    refused([&] { f.engine.prepare("replacement",f.before.selection,f.before.witness(),f.plan,102,f.reader()); },8);
    f.recover(); CHECK(f.record().phase == Phase::Placed);
    f.current = initial(Kind::Chair); ++f.current.next_building; ++f.current.next_job; ++f.current.building_count;
    ++f.current.selection.item; f.current.selection.target = {25,25,2};
    const auto c = f.engine.inspect(f.current.selection,f.reader());
    const auto p = plan_digest(c.selection,c.witness());
    const auto &r = f.engine.prepare("next",c.selection,c.witness(),p,102,f.reader());
    const auto &placed = f.engine.commit("next",p,r.token,103,f.reader(),[&](const Capture &v) {
        ++f.writers; f.current = v.expected_after();
    },f.verifier());
    CHECK(placed.phase == Phase::Placed && !f.engine.unresolved()); CHECK(f.writers == 2 && f.engine.size() == 2);
}
void no_effect_is_not_proof_of_nonapplication() {
    Fixture f; f.attempt(3); const auto bytes = f.record().encode();
    for (unsigned i = 0; i < 32; ++i) { f.recover(); f.unchanged(bytes); }
    f.engine.cancel("original",f.plan,f.token); f.unchanged(bytes);
    f.engine.commit("original",f.plan,f.token,999999,f.reader(),[&](const Capture &) { ++f.writers; },f.verifier());
    f.unchanged(bytes);
}
void complete_capture_must_match() {
    const std::vector<std::function<void(Capture &)>> changes = {
        [](auto &c) { ++c.tick; }, [](auto &c) { --c.tick; }, [](auto &c) { c.paused = false; },
        [](auto &c) { ++c.site; }, [](auto &c) { c.folder = "other"; }, [](auto &c) { ++c.dimensions[0]; },
        [](auto &c) { ++c.next_building; }, [](auto &c) { ++c.next_job; }, [](auto &c) { ++c.building_count; },
        [](auto &c) { c.free_tile = true; }, [](auto &c) { c.supported = false; },
        [](auto &c) { ++c.selection.item; }, [](auto &c) { ++c.selection.target.x; },
        [](auto &c) { c.item = Item{}; }, [](auto &c) { ++c.item.pos.x; },
        [](auto &c) { ++c.item.quality; }, [](auto &c) { ++c.item.wear; },
        [](auto &c) { ++c.item.native_type; }, [](auto &c) { ++c.item.subtype; },
        [](auto &c) { ++c.item.material; }, [](auto &c) { ++c.item.material_index; },
        [](auto &c) { c.item.other_flags = COMPUTED_ITEM_FLAGS; }, [](auto &c) { ++c.item.other_refs; },
        [](auto &c) { c.item.in_job = false; }, [](auto &c) { c.item.on_ground = false; },
        [](auto &c) { c.item.jobs.clear(); }, [](auto &c) { ++c.item.jobs[0]; },
        [](auto &c) { c.item.ground.liquid = 1; }, [](auto &c) { c.tiles[4].building.reset(); },
    };
    for (const auto &change : changes) {
        Fixture f; f.attempt(); const auto bytes = f.record().encode(); change(f.current);
        f.recover(); f.unchanged(bytes);
    }
    for (unsigned tile = 0; tile < 9; ++tile) for (unsigned field = 0; field < 6; ++field) {
        Fixture f; f.attempt(); const auto bytes = f.record().encode(); auto &t = f.current.tiles[tile];
        switch (field) {
            case 0: ++t.tiletype; break; case 1: t.liquid = 1; break; case 2: t.shape = 2; break;
            case 3: t.dig = 1; break; case 4: ++t.occupancy_other; break; default: t = Tile{};
        }
        f.recover(); f.unchanged(bytes);
    }
}
void exact_insertion_relationships_must_match() {
    for (unsigned field = 0; field < 14; ++field) {
        Fixture f; f.attempt(); const auto bytes = f.record().encode();
        f.engine.reconcile("original",f.plan,f.reader(),[&](const Capture &c) {
            auto p = insertion(c);
            switch (field) {
                case 0: ++p.building; break; case 1: ++p.job; break; case 2: ++p.item; break;
                case 3: p.kind = Kind::Chair; break; case 4: ++p.pos.x; break; case 5: ++p.material; break;
                case 6: ++p.material_index; break; case 7: p.stage = 1; break; case 8: p.max_stage = 0; break;
                case 9: p.max_stage = 33; break; case 10: p.linked = false; break;
                case 11: p.construct_job = false; break; case 12: p.exact_item_link = false; break;
                default: p.suspended = true;
            }
            return p;
        });
        f.unchanged(bytes);
    }
}
void repeated_failures_preserve_evidence_and_retry_only_reads() {
    for (unsigned fault = 0; fault < 3; ++fault) {
        Fixture f; f.attempt(); const auto bytes = f.record().encode();
        unsigned reads = 0;
        f.engine.reconcile("original",f.plan,[&](const Selection &) {
            ++reads; if ((fault == 0 && reads == 1) || (fault == 2 && reads == 2)) throw std::bad_alloc();
            return f.current;
        },[&](const Capture &c) { if (fault == 1) throw std::bad_alloc(); return insertion(c); });
        f.unchanged(bytes); f.recover(); CHECK(f.record().phase == Phase::Placed); CHECK(f.writers == 1);
    }
}
void final_capture_is_not_optional() {
    Fixture f; f.attempt(); const auto bytes = f.record().encode();
    f.engine.reconcile("original",f.plan,f.reader(),[&](const Capture &c) {
        ++f.current.item.quality; return insertion(c);
    });
    f.unchanged(bytes);
}
void source_and_intervention_fences_precede_reads() {
    for (bool source : {false,true}) {
        Fixture f; f.attempt(); const auto bytes = f.record().encode(); const auto n = f.reads;
        if (source) f.engine.change_source(); else f.engine.interrupt();
        f.recover(); f.unchanged(bytes); CHECK(f.reads == n);
    }
}
void callback_changes_cannot_publish_a_mixed_generation() {
    for (bool source : {false,true}) for (bool during_read : {false,true}) {
        Fixture f; f.attempt(); const auto bytes = f.record().encode();
        const auto interfere = [&] { if (source) f.engine.change_source(); else f.engine.interrupt(); };
        f.engine.reconcile("original",f.plan,[&](const Selection &) {
            if (during_read) interfere();
            return f.current;
        },[&](const Capture &c) { if (!during_read) interfere(); return insertion(c); });
        f.unchanged(bytes);
    }
}
void identity_validation_precedes_native_callbacks() {
    Fixture f; f.attempt(); const auto n = f.reads; const auto bytes = f.record().encode();
    CHECK(!f.engine.reconcile("unknown",f.plan,f.reader(),f.verifier()));
    refused([&] { f.engine.reconcile("bad key",f.plan,f.reader(),f.verifier()); },3);
    refused([&] { f.engine.reconcile("original","short",f.reader(),f.verifier()); },3);
    refused([&] { f.engine.reconcile("original",std::string(32,'x'),f.reader(),f.verifier()); },7);
    CHECK(f.reads == n); f.unchanged(bytes);
}
void unattempted_records_never_reconcile_into_placements() {
    for (unsigned state = 0; state < 3; ++state) {
        Fixture f;
        if (state == 1) f.engine.cancel("original",f.plan,f.token);
        if (state == 2) f.engine.commit("original",f.plan,f.token,100 + PREPARE_MS,f.reader(),
            [&](const Capture &) { ++f.writers; },f.verifier());
        const auto bytes = f.record().encode(); const auto n = f.reads;
        f.current = f.before.expected_after();
        CHECK(f.recover()->encode() == bytes); CHECK(f.reads == n && f.writers == 0);
    }
}
void reentrancy_cannot_publish_or_lose_the_busy_guard() {
    Fixture f; f.attempt(); const auto bytes = f.record().encode();
    f.engine.reconcile("original",f.plan,f.reader(),[&](const Capture &c) {
        f.recover(); return insertion(c);
    });
    f.unchanged(bytes); f.recover(); CHECK(f.record().phase == Phase::Placed && f.writers == 1);
}
} // namespace
int main() {
    const std::pair<const char *, void (*)()> tests[] = {
        {"positive_recovery",positive_recovery_and_byte_identity},
        {"fence_release",old_query_dead_end_and_fence_release},
        {"no_effect",no_effect_is_not_proof_of_nonapplication},
        {"complete_capture",complete_capture_must_match},
        {"exact_links",exact_insertion_relationships_must_match},
        {"retry_reads",repeated_failures_preserve_evidence_and_retry_only_reads},
        {"final_capture",final_capture_is_not_optional},
        {"source_fences",source_and_intervention_fences_precede_reads},
        {"callback_changes",callback_changes_cannot_publish_a_mixed_generation},
        {"identity",identity_validation_precedes_native_callbacks},
        {"unattempted",unattempted_records_never_reconcile_into_placements},
        {"reentrancy",reentrancy_cannot_publish_or_lose_the_busy_guard},
    };
    for (const auto &t : tests) {
        try { t.second(); ++scenarios; }
        catch (const std::exception &e) { std::cerr << t.first << ": " << e.what() << '\n'; return 1; }
    }
    std::cout << "{\"scenarios\":" << scenarios << ",\"assertions\":" << checks
              << ",\"actual_engine\":true,\"native_callbacks\":\"injected\"}\n";
}
