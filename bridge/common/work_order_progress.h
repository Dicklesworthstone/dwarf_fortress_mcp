#pragma once
#include <algorithm>
#include <cstdint>
#include <exception>
#include <limits>
#include <string>
#include <vector>

// Selected read-only progress, NOT a production receipt or a mutation engine.
// The caller holds DFHack's suspension until the complete observation is copied.
namespace dfmcp_work_order_progress {
constexpr std::size_t MAX_TARGETS = 32, MAX_QUEUE = 4096, MAX_BYTES = 16 * 1024;
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t value) : code(value) {}
    const char *what() const noexcept override { return "work-order progress refused"; }
};
inline void require(bool value, std::uint32_t code = 3) { if (!value) throw Failure(code); }
inline bool utf8(const std::string &s, std::size_t maximum, bool empty = false) {
    if (s.size() > maximum || (!empty && s.empty())) return false;
    for (std::size_t i = 0; i < s.size();) {
        const auto c = static_cast<unsigned char>(s[i]);
        if (!c) return false;
        if (c < 128) { ++i; continue; }
        const std::size_t n = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
        if (!n || s.size() - i < n) return false;
        for (std::size_t k = 1; k < n; ++k)
            if ((static_cast<unsigned char>(s[i+k]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(s[i+1]);
        if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0)
            || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
inline void u32(std::string &s, std::uint32_t v) {
    for (int shift = 24; shift >= 0; shift -= 8) s.push_back(static_cast<char>(v >> shift));
}
inline void u64(std::string &s, std::uint64_t v) {
    for (int shift = 56; shift >= 0; shift -= 8) s.push_back(static_cast<char>(v >> shift));
}
inline void text(std::string &s, const std::string &v) {
    s.push_back(static_cast<char>(v.size() >> 8)); s.push_back(static_cast<char>(v.size())); s += v;
}
inline void targets(const std::vector<std::uint32_t> &ids) {
    require(!ids.empty() && ids.size() <= MAX_TARGETS);
    for (std::size_t i = 0; i < ids.size(); ++i)
        require(ids[i] <= INT32_MAX && (!i || ids[i-1] < ids[i]));
}
struct Row {
    std::uint32_t id = 0;
    bool present = false;
    // Recipe 0 means this is NOT one of the fully recognized static templates.
    std::uint8_t recipe = 0;
    std::int32_t job_type = 0, left = 0, total = 0, frequency = 0, workshop = 0, max_workshops = 0;
    std::int32_t next_check_year = 0, next_check_tick = 0;
    std::uint32_t status = 0, item_conditions = 0, order_conditions = 0;
    std::string type_key, reaction;
    std::string encode() const {
        require(id <= INT32_MAX, 5);
        std::string out; u32(out, id); out.push_back(present ? 1 : 0);
        if (!present) {
            require(recipe == 0 && job_type == 0 && left == 0 && total == 0 && frequency == 0
                && workshop == 0 && max_workshops == 0 && next_check_year == 0 && next_check_tick == 0
                && status == 0 && item_conditions == 0 && order_conditions == 0
                && type_key.empty() && reaction.empty(), 5);
            return out;
        }
        require(recipe <= 4 && job_type >= 0 && left >= INT16_MIN && left <= INT16_MAX
            && total >= INT16_MIN && total <= INT16_MAX && item_conditions <= MAX_QUEUE
            && order_conditions <= MAX_QUEUE && utf8(type_key, 128) && utf8(reaction, 128, true), 5);
        // Recognized means the static finite-wood template matches, not approval or success.
        require(recipe == 0 || (total >= 1 && total <= 100 && left >= 0 && left <= total
            && frequency == 0 && workshop == -1 && max_workshops == 1
            && item_conditions == 0 && order_conditions == 0 && reaction.empty() && (status & ~3u) == 0), 5);
        u32(out, static_cast<std::uint32_t>(job_type)); out.push_back(static_cast<char>(recipe));
        for (auto v : {left, total}) u32(out, static_cast<std::uint32_t>(v));
        u32(out, status);
        for (auto v : {frequency, workshop, max_workshops, next_check_year, next_check_tick})
            u32(out, static_cast<std::uint32_t>(v));
        u32(out, item_conditions); u32(out, order_conditions);
        text(out, type_key); text(out, reaction); return out;
    }
};
struct Observation {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t site = 0, next_order = 0, queue_count = 0;
    bool paused = false;
    std::string folder;
    std::vector<Row> rows;
    std::string encode() const {
        require(generation && generation != UINT64_MAX && sequence && sequence != UINT64_MAX
            && tick <= std::uint64_t{UINT32_MAX} * 403200 + 403199
            && site <= INT32_MAX && next_order <= INT32_MAX && queue_count <= MAX_QUEUE
            && queue_count <= next_order && utf8(folder, 512), 5);
        std::vector<std::uint32_t> ids; for (const auto &r : rows) ids.push_back(r.id); targets(ids);
        std::string out = "DFMWP012";
        for (auto v : {generation, sequence, tick}) u64(out, v);
        for (auto v : {site, next_order, queue_count}) u32(out, v);
        out.push_back(paused ? 1 : 0); text(out, folder); u32(out, static_cast<std::uint32_t>(rows.size()));
        std::size_t present_count = 0;
        for (const auto &r : rows) {
            if (r.present) { require(r.id < next_order, 5); ++present_count; }
            out += r.encode();
        }
        require(present_count <= queue_count && out.size() <= MAX_BYTES, 5); return out;
    }
};
class Sequence {
    std::uint64_t generation_, sequence_ = 0;
public:
    explicit Sequence(std::uint64_t generation) : generation_(generation) {}
    bool available() const { return generation_ && generation_ != UINT64_MAX && sequence_ < UINT64_MAX - 1; }
    std::uint64_t generation() const { return generation_; }
    void reset() { if (generation_ != UINT64_MAX) ++generation_; sequence_ = 0; }
    void stamp(Observation &out) {
        require(available(), 5); out.generation = generation_; out.sequence = ++sequence_;
    }
};
} // namespace dfmcp_work_order_progress
