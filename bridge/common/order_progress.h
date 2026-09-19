#pragma once
#include <cstdint>
#include <stdexcept>
#include <string>
#include <string_view>

// A selected manager-order observation. No mutation, native pointer retention,
// allocation authority, or goods-production/completion claim lives here.
namespace dfmcp_order_progress {
constexpr std::size_t MAX_ORDERS = 4096;
constexpr std::size_t MAX_BYTES = 1024;
struct Failure : std::exception {
    std::uint32_t code;
    explicit Failure(std::uint32_t value) : code(value) {}
    const char *what() const noexcept override { return "order progress refused"; }
};
inline void require(bool value, std::uint32_t code = 3) { if (!value) throw Failure(code); }
inline void u32(std::string &out, std::uint32_t value) {
    for (int shift = 24; shift >= 0; shift -= 8) out.push_back(static_cast<char>(value >> shift));
}
inline void u64(std::string &out, std::uint64_t value) {
    for (int shift = 56; shift >= 0; shift -= 8) out.push_back(static_cast<char>(value >> shift));
}
inline bool utf8(std::string_view value, std::size_t maximum) {
    if (value.empty() || value.size() > maximum) return false;
    for (std::size_t i = 0; i < value.size();) {
        const auto c = static_cast<unsigned char>(value[i]);
        if (!c) return false;
        if (c < 128) { ++i; continue; }
        const std::size_t n = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
        if (!n || value.size() - i < n) return false;
        for (std::size_t k = 1; k < n; ++k)
            if ((static_cast<unsigned char>(value[i+k]) & 0xc0) != 0x80) return false;
        const auto b = static_cast<unsigned char>(value[i+1]);
        if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0)
            || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) return false;
        i += n;
    }
    return true;
}
struct Observation {
    std::uint64_t generation = 0, sequence = 0, tick = 0;
    std::uint32_t order = 0, next_order = 0, site = 0;
    bool paused = false, present = false;
    std::string folder;
    std::int32_t job_type = 0, frequency = 0;
    std::uint32_t left = 0, total = 0, status = 0;
    // Zero means not a fully verified member of the finite furniture template.
    // Nonzero values use work-orders/1.10's four semantic recipe codes.
    std::uint8_t recipe = 0;
    std::string encode() const {
        require(generation && generation != UINT64_MAX && sequence && sequence != UINT64_MAX, 5);
        require(tick <= std::uint64_t{UINT32_MAX} * 403200 + 403199 && order <= INT32_MAX
            && next_order <= INT32_MAX && site <= INT32_MAX && utf8(folder,512), 5);
        if (present) {
            require(order < next_order && job_type >= 0 && frequency >= -1
                && left <= INT16_MAX && total <= INT16_MAX && recipe <= 4, 5);
            if (recipe) require(total >= 1 && total <= 100 && left <= total && frequency == 0 && !(status & ~3u), 5);
        } else {
            require(job_type == 0 && frequency == 0 && left == 0 && total == 0 && status == 0
                && recipe == 0, 5);
        }
        std::string out = "DFMOP011";
        u64(out,generation); u64(out,sequence); u64(out,tick);
        u32(out,order); u32(out,next_order); u32(out,site);
        out.push_back(paused ? 1 : 0); out.push_back(present ? 1 : 0);
        out.push_back(static_cast<char>(folder.size() >> 8)); out.push_back(static_cast<char>(folder.size())); out += folder;
        if (present) {
            u32(out,static_cast<std::uint32_t>(job_type)); u32(out,left); u32(out,total); u32(out,status);
            u32(out,static_cast<std::uint32_t>(frequency)); out.push_back(static_cast<char>(recipe));
        }
        require(out.size() <= MAX_BYTES,5); return out;
    }
};
class Reader {
    std::uint64_t generation_, sequence_ = 0;
public:
    explicit Reader(std::uint64_t generation) : generation_(generation) {}
    std::uint64_t generation() const { return generation_; }
    bool available() const { return generation_ && generation_ != UINT64_MAX && sequence_ < UINT64_MAX - 1; }
    void reset() { if (generation_ != UINT64_MAX) ++generation_; sequence_ = 0; }
    template<class Capture> Observation read(std::uint32_t order, Capture capture) {
        require(order <= INT32_MAX); require(available(),5);
        auto out = capture(order); require(out.order == order,5);
        out.generation = generation_; out.sequence = ++sequence_;
        (void)out.encode(); return out;
    }
};
}
