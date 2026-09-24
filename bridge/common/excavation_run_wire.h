#pragma once
#include "excavation_run.h"
#include "bounded_run_wire.h"

// Independent excavation-run/1.18 domains. Embedded DFMRO013 bytes describe
// clock fields only; they never import the authority of a run/1.13 connection.
namespace dfmcp_excavation_run {
constexpr std::size_t MAX_CAPTURE_BYTES = 1024, MAX_RECORD_BYTES = 3072;
inline void u16(std::string &out, std::size_t n) {
    require(n <= UINT16_MAX); out.push_back(static_cast<char>(n >> 8)); out.push_back(static_cast<char>(n));
}
inline void region_bytes(std::string &out, Region r) {
    r.validate(); for (auto n : {r.x, r.y, r.z, r.width, r.height}) clock::append_u32(out, n);
}
inline std::string encode_capture(const Capture &value) {
    value.validate();
    std::string out = "DFMEC018"; out += clock::encode_snapshot(value.source.clock);
    const auto &id = value.source.identity;
    for (auto n : {id.site, id.map_x, id.map_y, id.map_z}) clock::append_u32(out, n);
    u16(out, id.folder.size()); out += id.folder;
    region_bytes(out, value.region); u16(out, value.region.size());
    for (std::size_t i = 0; i < value.region.size(); ++i) {
        const auto cell = value.cells[i]; out.push_back(static_cast<char>(cell.presence));
        if (cell.presence == 2) {
            out.push_back(static_cast<char>(cell.shape)); out.push_back(static_cast<char>(cell.liquid));
            out.push_back(static_cast<char>(cell.dig));
        }
    }
    require(out.size() <= MAX_CAPTURE_BYTES, 5); return out;
}
class CaptureDecoder {
public:
    explicit CaptureDecoder(const std::string &bytes) : bytes_(bytes) { require(bytes.size() <= MAX_CAPTURE_BYTES); }
    std::string take(std::size_t n) {
        require(n <= bytes_.size() - offset_); const auto start = offset_; offset_ += n; return bytes_.substr(start, n);
    }
    std::uint32_t word(unsigned n) {
        require(n <= 4 && n <= bytes_.size() - offset_);
        std::uint32_t out = 0;
        while (n--) out = (out << 8) | static_cast<unsigned char>(bytes_[offset_++]);
        return out;
    }
    bool done() const { return offset_ == bytes_.size(); }
private:
    const std::string &bytes_;
    std::size_t offset_ = 0;
};
inline Capture decode_capture(const std::string &bytes) {
    CaptureDecoder in(bytes); require(in.take(8) == "DFMEC018");
    Capture out; out.source.clock = clock::decode_snapshot(in.take(35));
    auto &id = out.source.identity; id.generation = out.source.clock.generation;
    id.site = in.word(4); id.map_x = in.word(4); id.map_y = in.word(4); id.map_z = in.word(4);
    const auto length = in.word(2); require(length >= 1 && length <= 512); id.folder = in.take(length);
    out.region = {in.word(4), in.word(4), in.word(4), in.word(4), in.word(4)};
    const auto count = in.word(2); require(count == out.region.size());
    for (std::size_t i = 0; i < count; ++i) {
        auto &cell = out.cells[i]; cell.presence = static_cast<std::uint8_t>(in.word(1));
        require(cell.presence <= 2);
        if (cell.presence == 2) {
            cell.shape = static_cast<std::uint8_t>(in.word(1)); cell.liquid = static_cast<std::uint8_t>(in.word(1));
            cell.dig = static_cast<std::uint8_t>(in.word(1));
        }
    }
    require(in.done()); out.validate(); require(encode_capture(out) == bytes); return out;
}
inline void spec_bytes(std::string &out, clock::Spec limits, Goal goal) {
    goal.validate(limits);
    for (auto n : {limits.game_ticks, limits.wall_ms, goal.samples, goal.stable_ticks, goal.interval, goal.max_gap})
        clock::append_u32(out, n);
}
inline std::string plan_digest(clock::Spec limits, Goal goal, const Capture &before) {
    std::string out; spec_bytes(out, limits, goal); out += encode_capture(before);
    return clock::hash_domain("dfmcp-excavation-run-plan/1", out);
}
inline std::string token_for(const std::string &key, const std::string &plan) {
    require(plan.size() == 32); std::string out; clock::append_key(out, key); out += plan;
    return clock::hash_domain("dfmcp-excavation-run-token/1", out).substr(0, 16);
}
inline void validate_record(const Record &record) {
    require(record.run, 5); const auto &run = *record.run;
    record.before.validate(); record.goal.validate(run.spec);
    require(run.before == record.before.source.clock && run.before.paused && !record.before.matches()
        && run.before.sequence != UINT64_MAX && run.before.tick <= MAX_TICK - run.spec.game_ticks, 5);
    for (std::size_t i = 0; i < record.before.region.size(); ++i)
        require(record.before.cells[i].presence == 2 && !record.before.cells[i].liquid, 5);
    const auto phase = static_cast<unsigned>(run.phase), reason = static_cast<unsigned>(run.reason);
    require(phase <= 5 && reason <= 9 && static_cast<unsigned>(record.trigger) <= 5, 5);
    require(run.unpause_attempted == (phase == 1 || phase == 2 || phase == 3 || phase == 5)
        && run.pause_verified == (phase == 3) && (!run.tick_known || run.observed_tick <= MAX_TICK), 5);
    if (phase == 0) require(reason == 0 && !run.tick_known, 5);
    if (phase == 1) require(reason == 0 && run.tick_known && run.observed_tick >= run.before.tick, 5);
    if (phase == 2 || phase == 3) require(reason == 1 || reason == 2 || reason == 3 || reason == 5
        || reason == 6 || reason == 8 || (phase == 3 && reason == 4), 5);
    if (phase == 4) require((reason == 3 || reason == 7 || reason == 9) && !run.tick_known, 5);
    if (phase == 5) require(reason == 7 && !run.tick_known, 5);
    require(record.stable_samples <= record.goal.samples && record.last_tick >= run.before.tick
        && record.counted_tick >= run.before.tick && record.counted_tick <= record.last_tick, 5);
    require(record.stable_samples ? record.first_stable_tick > run.before.tick
        && record.first_stable_tick <= record.counted_tick : record.first_stable_tick == 0, 5);
    if (record.sample) {
        const auto &sample = *record.sample; sample.validate();
        require(sample.region == record.before.region && sample.source.identity == record.before.source.identity
            && sample.source.clock.sequence == run.before.sequence + 1 && !sample.source.clock.paused
            && sample.source.clock.tick == record.last_tick && record.last_tick < run.before.tick + run.spec.game_ticks, 5);
    } else require(record.stable_samples == 0 && record.last_tick == run.before.tick && record.counted_tick == run.before.tick, 5);
    if (phase == 0 || phase == 4) require(!record.sample && record.trigger == Trigger::None, 5);
    if (record.trigger != Trigger::None) require(phase == 2 || phase == 3 || phase == 5, 5);
    if (record.trigger == Trigger::FloorObserved) require(record.sample && record.sample->matches()
        && record.stable_samples == record.goal.samples && record.counted_tick == record.last_tick
        && record.counted_tick - record.first_stable_tick >= record.goal.stable_ticks, 5);
}
inline std::string encode_record(const Record &record) {
    validate_record(record); const auto &run = *record.run;
    require(run.plan == plan_digest(run.spec, record.goal, record.before), 7);
    std::string out = "DFMER018"; clock::append_key(out, run.key); spec_bytes(out, run.spec, record.goal);
    const auto before = encode_capture(record.before); u16(out, before.size()); out += before;
    out += run.plan; out += token_for(run.key, run.plan);
    out.push_back(static_cast<char>(run.phase)); out.push_back(static_cast<char>(run.reason));
    out.push_back(run.unpause_attempted ? 1 : 0); out.push_back(run.pause_verified ? 1 : 0);
    out.push_back(run.tick_known ? 1 : 0); clock::append_u64(out, run.tick_known ? run.observed_tick : 0);
    out.push_back(static_cast<char>(record.trigger)); clock::append_u32(out, record.stable_samples);
    for (auto n : {record.first_stable_tick, record.counted_tick, record.last_tick}) clock::append_u64(out, n);
    out.push_back(record.sample ? 1 : 0);
    if (record.sample) { const auto sample = encode_capture(*record.sample); u16(out, sample.size()); out += sample; }
    out += clock::hash_domain("dfmcp-excavation-run-receipt/1", out);
    require(out.size() <= MAX_RECORD_BYTES, 5); return out;
}
} // namespace dfmcp_excavation_run
