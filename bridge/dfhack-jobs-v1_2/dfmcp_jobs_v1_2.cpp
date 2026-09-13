#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <string>
#include <string_view>
#include <vector>
#include "Core.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "MiscUtils.h"
#include "DfmcpJobsV1_2.pb.h"
#include "modules/Job.h"
#include "modules/World.h"
#include "df/building.h"
#include "df/global_objects.h"
#include "df/job.h"
#include "df/job_list_link.h"
#include "df/unit.h"
#include "df/world.h"

using namespace DFHack;
namespace wire = dfmcp::jobs::v1_2;
DFHACK_PLUGIN("dfmcp_jobs_v1_2");

namespace {
constexpr std::size_t MAX_JOBS = 4096;
constexpr std::size_t MAX_PAYLOAD = 2 * 1024 * 1024;
std::uint64_t generation = static_cast<std::uint64_t>(
    std::chrono::steady_clock::now().time_since_epoch().count()) | std::uint64_t{1};

bool valid_utf8(const std::string &text, std::size_t maximum, bool empty)
{
    if (text.size() > maximum || (!empty && text.empty())) return false;
    std::size_t i = 0;
    while (i < text.size()) {
        const auto c = static_cast<unsigned char>(text[i]);
        if (c == 0) return false;
        if (c < 0x80) { ++i; continue; }
        const std::size_t width = c >= 0xC2 && c <= 0xDF ? 2 :
            c >= 0xE0 && c <= 0xEF ? 3 : c >= 0xF0 && c <= 0xF4 ? 4 : 0;
        if (!width || i + width > text.size()) return false;
        for (std::size_t j = 1; j < width; ++j)
            if ((static_cast<unsigned char>(text[i+j]) & 0xC0) != 0x80) return false;
        const auto second = static_cast<unsigned char>(text[i+1]);
        if ((c == 0xE0 && second < 0xA0) || (c == 0xED && second >= 0xA0) ||
            (c == 0xF0 && second < 0x90) || (c == 0xF4 && second >= 0x90)) return false;
        i += width;
    }
    return true;
}

void u32(std::string &out, std::uint32_t value)
{
    for (int shift = 24; shift >= 0; shift -= 8)
        out.push_back(static_cast<char>((value >> shift) & 255));
}
void i32(std::string &out, std::int32_t value) { u32(out, static_cast<std::uint32_t>(value)); }
void text(std::string &out, const std::string &value)
{
    out.push_back(static_cast<char>((value.size() >> 8) & 255));
    out.push_back(static_cast<char>(value.size() & 255));
    out.append(value);
}
void reference(std::string &out, std::int32_t id)
{
    out.push_back(id >= 0 ? 1 : 0);
    if (id >= 0) u32(out, static_cast<std::uint32_t>(id));
}

bool initialize_and_authorize(const wire::Request *in, wire::Reply *out)
{
    out->Clear();
    out->set_accepted(false);
    out->set_failure_code(3);
    out->set_client_nonce("");
    out->set_protocol_major(1);
    out->set_protocol_minor(2);
    out->set_bridge_generation(0);
    out->set_df_version("");
    out->set_dfhack_version("");
    if (in->client_nonce().size() < 16 || in->client_nonce().size() > 64 ||
        !in->max_jobs() || in->max_jobs() > MAX_JOBS) return false;
    out->set_client_nonce(in->client_nonce());
    if (in->protocol_major() != 1 || in->protocol_minor() != 2) {
        out->set_failure_code(2); return false;
    }
    const char *configured = std::getenv("DFMCP_JOBS_TOKEN");
    const std::string_view expected = configured ? std::string_view(configured) : std::string_view();
    const std::string &presented = in->bearer_token();
    out->set_failure_code(1);
    if (expected.size() < 32 || expected.size() > 256 || presented.size() < 32 || presented.size() > 256)
        return false;
    std::size_t difference = expected.size() ^ presented.size();
    for (std::size_t i = 0; i < 256; ++i) {
        const unsigned char a = i < expected.size() ? expected[i] : 0;
        const unsigned char b = i < presented.size() ? presented[i] : 0;
        difference |= static_cast<std::size_t>(a ^ b);
    }
    if (difference) return false;
    out->set_failure_code(5);
    const auto &version = Core::getInstance().vinfo;
    const std::string df_version = version ? version->getVersion() : std::string();
    const std::string dfhack_version = Version::dfhack_version();
    if (!generation || generation == std::numeric_limits<std::uint64_t>::max() ||
        !valid_utf8(df_version, 128, false) || !valid_utf8(dfhack_version, 128, false)) return false;
    out->set_bridge_generation(generation);
    out->set_df_version(df_version);
    out->set_dfhack_version(dfhack_version);
    out->set_failure_code(0);
    return true;
}

command_result Handshake(color_ostream &, const wire::Request *in, wire::Reply *out)
{
    if (initialize_and_authorize(in, out)) out->set_accepted(true);
    return CR_OK;
}

command_result ReadObservation(color_ostream &, const wire::Request *in, wire::Reply *out)
{
    if (!initialize_and_authorize(in, out)) return CR_OK;
    out->set_failure_code(4);
    if (!Core::getInstance().isWorldLoaded() || !World::isFortressMode() ||
        !df::global::world || !df::global::job_next_id) return CR_OK;
    const auto year = static_cast<std::int64_t>(World::ReadCurrentYear());
    const auto year_tick = World::ReadCurrentTick();
    const auto site = World::GetCurrentSiteId();
    const auto next_id = *df::global::job_next_id;
    const std::string folder = World::ReadWorldFolder();
    out->set_failure_code(5);
    if (year < 0 || year > std::numeric_limits<std::uint32_t>::max() ||
        year_tick >= 403200 || site < 0 || next_id < 0 ||
        !valid_utf8(folder, 512, false)) return CR_OK;

    // RPC functions registered with flags=0 execute under DFHack's suspension.
    // Walk at most max_jobs+1 links; cyclic, oversized, or incomplete lists fail.
    std::vector<df::job *> jobs;
    jobs.reserve(in->max_jobs());
    for (auto *link = df::global::world->jobs.list.next; link; link = link->next) {
        if (jobs.size() >= in->max_jobs()) { out->set_failure_code(3); return CR_OK; }
        if (!link->item) return CR_OK;
        jobs.push_back(link->item);
    }
    std::sort(jobs.begin(), jobs.end(), [](const df::job *a, const df::job *b) { return a->id < b->id; });
    std::string payload("DFMJ1200", 8);
    u32(payload, static_cast<std::uint32_t>(year));
    u32(payload, year_tick);
    payload.push_back(World::ReadPauseState() ? 1 : 0);
    i32(payload, site);
    u32(payload, static_cast<std::uint32_t>(next_id));
    text(payload, folder);
    u32(payload, static_cast<std::uint32_t>(jobs.size()));
    std::int32_t previous = -1;
    for (auto *job : jobs) {
        if (job->id <= previous || job->id >= next_id || static_cast<int>(job->job_type) < 0 ||
            job->completion_timer < -1 || job->items.size() > 65536 ||
            job->job_items.elements.size() > 4096 || job->general_refs.size() > 4096) return CR_OK;
        for (const auto *ref : job->general_refs) if (!ref) return CR_OK;
        const std::string type_key = ENUM_KEY_STR(job_type, job->job_type);
        if (!valid_utf8(type_key, 128, false) || !valid_utf8(job->reaction_name, 128, true)) return CR_OK;
        auto *worker = Job::getWorker(job);
        auto *holder = Job::getHolder(job);
        if ((worker && worker->id < 0) || (holder && holder->id < 0)) return CR_OK;
        u32(payload, static_cast<std::uint32_t>(job->id));
        i32(payload, static_cast<std::int32_t>(job->job_type));
        text(payload, type_key);
        text(payload, job->reaction_name);
        payload.push_back(job->flags.bits.suspend ? 1 : 0);
        payload.push_back(job->flags.bits.repeat ? 1 : 0);
        i32(payload, job->pos.x); i32(payload, job->pos.y); i32(payload, job->pos.z);
        reference(payload, worker ? worker->id : -1);
        reference(payload, holder ? holder->id : -1);
        i32(payload, job->completion_timer);
        u32(payload, static_cast<std::uint32_t>(job->items.size()));
        u32(payload, static_cast<std::uint32_t>(job->job_items.elements.size()));
        if (payload.size() > MAX_PAYLOAD) { out->set_failure_code(3); return CR_OK; }
        previous = job->id;
    }
    // No partial payload is ever put in the RPC response.
    out->set_observation(payload);
    out->set_failure_code(0);
    out->set_accepted(true);
    return CR_OK;
}
} // namespace

DFhackCExport command_result plugin_init(color_ostream &, std::vector<PluginCommand> &)
{
    return CR_OK;
}
DFhackCExport command_result plugin_shutdown(color_ostream &) { return CR_OK; }
DFhackCExport command_result plugin_onstatechange(color_ostream &, state_change_event event)
{
    if ((event == SC_WORLD_LOADED || event == SC_WORLD_UNLOADED) &&
        generation != std::numeric_limits<std::uint64_t>::max()) ++generation;
    return CR_OK;
}
DFhackCExport RPCService *plugin_rpcconnect(color_ostream &)
{
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0);
    service->addFunction("ReadObservation", ReadObservation, 0);
    return service;
}
