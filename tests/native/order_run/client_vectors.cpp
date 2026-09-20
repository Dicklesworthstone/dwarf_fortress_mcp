#include "bridge/common/order_run_wire.h"
#include <iostream>
#include <stdexcept>
using namespace dfmcp_order_run;
std::string hex_bytes(const std::string &data) {
    const char *digits = "0123456789abcdef"; std::string out;
    for (unsigned char c : data) { out += digits[c >> 4]; out += digits[c & 15]; }
    return out;
}
int main() {
    for (int mode = 0; mode < 15; ++mode) {
        Engine engine;
        Capture value{{41,0,100,true,true,true},{41,7,"fort"},9,10,true,1,10,10,0};
        bool fail_pause = false, fail_read = false;
        auto read = [&](std::uint32_t) { if (fail_read) throw clock::Failure(5); return value; };
        auto write = [&](const Identity &i, bool p) {
            require(i == value.identity, 4); if (p && fail_pause) throw clock::Failure(5); value.clock.paused = p;
        };
        auto now = Clock::time_point{std::chrono::seconds(100)};
        const clock::Spec limits{100,1000};
        const Goal goal{mode == 2 ? Predicate::Active : mode == 3 ? Predicate::RemainingAtMost : Predicate::Approved,
            mode == 3 ? 5u : 0u, 1, 1};
        const std::string key = "vector"; const auto digest = plan_digest(limits, goal, value);
        const auto &record = engine.prepare(key, digest, limits, goal, value, now, read);
        if (mode == 4) --value.left; // exact witness drift before commit
        if (mode != 0) engine.commit(key, digest, now, read, write);
        if (mode == 1 || mode == 2 || mode == 3 || mode == 5 || mode == 14) {
            value.status = mode == 2 ? 2 : 1; if (mode == 3) value.left = 5;
            if (mode == 5 || mode == 14) fail_pause = true;
        }
        if (mode == 6) value.identity.folder = "replacement";
        if (mode == 7) value.clock.tick = 199;
        if (mode == 8) now += std::chrono::seconds(1);
        if (mode == 9) value.clock.paused = true;
        if (mode == 10) { value.present = false; value.recipe = 0; value.total = value.left = 0; }
        if (mode == 11) value.recipe = 2;
        if (mode == 12) fail_read = true;
        if (mode != 0 && mode != 4 && mode != 13) { ++value.clock.tick; engine.service(now, read, write); }
        if (mode == 14) { value.identity.site = 8; engine.service(now, read, write); }
        std::cout << mode << '=' << hex_bytes(encode_record(record)) << '\n';
    }
    return 0;
}
