#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <string>
#include <string_view>
#include <vector>

#include "Core.h"
#include "Console.h"
#include "DfmcpBridge.pb.h"
#include "Export.h"
#include "MiscUtils.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/Translation.h"
#include "modules/Units.h"
#include "modules/World.h"

#include "df/coord.h"
#include "df/global_objects.h"
#include "df/report.h"
#include "df/unit.h"
#include "df/world.h"

using namespace DFHack;
namespace wire = dfmcp::bridge::v1;

DFHACK_PLUGIN("dfmcp_bridge");

namespace {

constexpr std::uint32_t PROTOCOL_MAJOR = 1;
constexpr std::uint32_t CITIZEN_PROTOCOL_MINOR = 0;
constexpr std::uint32_t ANNOUNCEMENT_PROTOCOL_MINOR = 1;
constexpr const char *BRIDGE_VERSION = "0.2.0";
constexpr std::size_t MIN_TOKEN_BYTES = 32;
constexpr std::size_t MAX_TOKEN_BYTES = 256;
constexpr std::size_t MIN_NONCE_BYTES = 16;
constexpr std::size_t MAX_NONCE_BYTES = 64;
constexpr std::size_t MAX_CLIENT_NAME_BYTES = 128;
constexpr std::size_t MAX_CLIENT_VERSION_BYTES = 64;
constexpr std::uint32_t DEFAULT_MAX_CITIZENS = 256;
constexpr std::uint32_t HARD_MAX_CITIZENS = 4096;
constexpr std::uint32_t DEFAULT_MAX_ANNOUNCEMENTS = 256;
constexpr std::uint32_t HARD_MAX_ANNOUNCEMENTS = 4096;
constexpr std::size_t MAX_UNIT_NAME_BYTES = 256;
constexpr std::size_t MAX_RACE_NAME_BYTES = 128;
constexpr std::size_t MAX_WORLD_NAME_BYTES = 256;
constexpr std::size_t MAX_WORLD_FOLDER_BYTES = 512;
constexpr std::size_t MAX_ANNOUNCEMENT_TEXT_BYTES = 2048;
constexpr std::uint32_t TICKS_PER_DAY = 1200;
constexpr std::uint32_t DAYS_PER_MONTH = 28;
constexpr std::uint32_t MONTHS_PER_YEAR = 12;
constexpr std::uint32_t TICKS_PER_YEAR =
    TICKS_PER_DAY * DAYS_PER_MONTH * MONTHS_PER_YEAR;

const std::uint64_t BRIDGE_GENERATION =
    static_cast<std::uint64_t>(
        std::chrono::steady_clock::now().time_since_epoch().count()) |
    std::uint64_t{1};

bool is_continuation(unsigned char byte)
{
    return (byte & 0xC0U) == 0x80U;
}

std::size_t valid_utf8_width(const std::string &value, std::size_t offset)
{
    const auto lead = static_cast<unsigned char>(value[offset]);
    if (lead <= 0x7FU)
        return 1;
    if (lead >= 0xC2U && lead <= 0xDFU)
    {
        if (offset + 2 > value.size())
            return 0;
        return is_continuation(static_cast<unsigned char>(value[offset + 1])) ? 2 : 0;
    }
    if (lead >= 0xE0U && lead <= 0xEFU)
    {
        if (offset + 3 > value.size())
            return 0;
        const auto second = static_cast<unsigned char>(value[offset + 1]);
        const auto third = static_cast<unsigned char>(value[offset + 2]);
        if (!is_continuation(second) || !is_continuation(third))
            return 0;
        if (lead == 0xE0U && second < 0xA0U)
            return 0;
        if (lead == 0xEDU && second > 0x9FU)
            return 0;
        return 3;
    }
    if (lead >= 0xF0U && lead <= 0xF4U)
    {
        if (offset + 4 > value.size())
            return 0;
        const auto second = static_cast<unsigned char>(value[offset + 1]);
        const auto third = static_cast<unsigned char>(value[offset + 2]);
        const auto fourth = static_cast<unsigned char>(value[offset + 3]);
        if (!is_continuation(second) || !is_continuation(third) ||
            !is_continuation(fourth))
            return 0;
        if (lead == 0xF0U && second < 0x90U)
            return 0;
        if (lead == 0xF4U && second > 0x8FU)
            return 0;
        return 4;
    }
    return 0;
}

std::string bounded_utf8_prefix(const std::string &value, std::size_t max_bytes)
{
    std::size_t offset = 0;
    while (offset < value.size() && offset < max_bytes)
    {
        const std::size_t width = valid_utf8_width(value, offset);
        if (width == 0 || offset + width > max_bytes)
            break;
        offset += width;
    }
    return value.substr(0, offset);
}

bool constant_time_equal(std::string_view left, std::string_view right)
{
    std::size_t difference = left.size() ^ right.size();
    for (std::size_t index = 0; index < MAX_TOKEN_BYTES; ++index)
    {
        const auto left_byte = index < left.size()
            ? static_cast<unsigned char>(left[index])
            : static_cast<unsigned char>(0);
        const auto right_byte = index < right.size()
            ? static_cast<unsigned char>(right[index])
            : static_cast<unsigned char>(0);
        difference |= static_cast<std::size_t>(left_byte ^ right_byte);
    }
    return difference == 0;
}

bool authenticate(const std::string &presented, std::string &failure_code,
                  std::string &failure_message)
{
    const char *configured = std::getenv("DFMCP_BRIDGE_TOKEN");
    if (!configured)
    {
        failure_code = "AUTH_REQUIRED";
        failure_message =
            "DFMCP_BRIDGE_TOKEN is not configured in the DFHack process";
        return false;
    }

    const std::string_view expected(configured);
    if (expected.size() < MIN_TOKEN_BYTES || expected.size() > MAX_TOKEN_BYTES)
    {
        failure_code = "AUTH_REQUIRED";
        failure_message =
            "configured bridge token violates the 32..256 byte policy";
        return false;
    }
    if (presented.size() < MIN_TOKEN_BYTES || presented.size() > MAX_TOKEN_BYTES)
    {
        failure_code = "AUTH_FAILED";
        failure_message = "presented bridge token violates the size policy";
        return false;
    }
    if (!constant_time_equal(expected, presented))
    {
        failure_code = "AUTH_FAILED";
        failure_message = "bridge authentication failed";
        return false;
    }
    return true;
}

bool supported_protocol(std::uint32_t major, std::uint32_t minor)
{
    return major == PROTOCOL_MAJOR &&
        (minor == CITIZEN_PROTOCOL_MINOR ||
         minor == ANNOUNCEMENT_PROTOCOL_MINOR);
}

bool validate_protocol(std::uint32_t major, std::uint32_t minor,
                       std::string &failure_code,
                       std::string &failure_message)
{
    if (!supported_protocol(major, minor))
    {
        failure_code = "PROTOCOL_MISMATCH";
        failure_message = "bridge protocol must be exactly 1.0 or 1.1";
        return false;
    }
    return true;
}

bool validate_announcement_protocol(std::uint32_t major, std::uint32_t minor,
                                    std::string &failure_code,
                                    std::string &failure_message)
{
    if (major != PROTOCOL_MAJOR || minor != ANNOUNCEMENT_PROTOCOL_MINOR)
    {
        failure_code = "PROTOCOL_MISMATCH";
        failure_message = "ReadAnnouncements requires bridge protocol 1.1";
        return false;
    }
    return true;
}

std::string df_version()
{
    const auto &version_info = Core::getInstance().vinfo;
    return version_info ? version_info->getVersion() : std::string("unknown");
}

void initialize_handshake_reply(wire::HandshakeReply *out,
                                std::uint32_t requested_minor)
{
    out->set_accepted(false);
    out->set_failure_code("");
    out->set_failure_message("");
    out->set_protocol_major(PROTOCOL_MAJOR);
    out->set_protocol_minor(
        supported_protocol(PROTOCOL_MAJOR, requested_minor)
            ? requested_minor
            : CITIZEN_PROTOCOL_MINOR);
    out->set_bridge_version("");
    out->set_dfhack_version("");
    out->set_df_version("");
    out->set_world_loaded(false);
    out->set_fortress_mode(false);
    out->set_client_nonce("");
    out->set_bridge_generation(0);
}

void publish_handshake_manifest(wire::HandshakeReply *out,
                                std::uint32_t protocol_minor)
{
    out->set_bridge_version(BRIDGE_VERSION);
    out->set_dfhack_version(Version::dfhack_version());
    out->set_df_version(df_version());
    const bool world_loaded = Core::getInstance().isWorldLoaded();
    out->set_world_loaded(world_loaded);
    out->set_fortress_mode(world_loaded && World::isFortressMode());
    out->set_bridge_generation(BRIDGE_GENERATION);
    out->add_supported_methods("Handshake");
    out->add_supported_methods("ReadObservation");
    if (protocol_minor == ANNOUNCEMENT_PROTOCOL_MINOR)
        out->add_supported_methods("ReadAnnouncements");
}

command_result Handshake(color_ostream &, const wire::HandshakeRequest *in,
                         wire::HandshakeReply *out)
{
    initialize_handshake_reply(out, in->protocol_minor());

    std::string failure_code;
    std::string failure_message;
    if (in->client_name().empty() ||
        in->client_name().size() > MAX_CLIENT_NAME_BYTES ||
        in->client_version().empty() ||
        in->client_version().size() > MAX_CLIENT_VERSION_BYTES ||
        in->client_nonce().size() < MIN_NONCE_BYTES ||
        in->client_nonce().size() > MAX_NONCE_BYTES)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message(
            "client name, version, or nonce violates the handshake bounds");
        return CR_OK;
    }
    out->set_client_nonce(in->client_nonce());
    if (!validate_protocol(in->protocol_major(), in->protocol_minor(),
                           failure_code, failure_message) ||
        !authenticate(in->bearer_token(), failure_code, failure_message))
    {
        out->set_failure_code(failure_code);
        out->set_failure_message(failure_message);
        return CR_OK;
    }

    publish_handshake_manifest(out, in->protocol_minor());
    out->set_accepted(true);
    return CR_OK;
}

void initialize_observation_reply(wire::ReadObservationReply *out,
                                  std::uint32_t requested_minor)
{
    out->set_accepted(false);
    out->set_failure_code("");
    out->set_failure_message("");
    out->set_protocol_major(PROTOCOL_MAJOR);
    out->set_protocol_minor(
        supported_protocol(PROTOCOL_MAJOR, requested_minor)
            ? requested_minor
            : CITIZEN_PROTOCOL_MINOR);
    out->set_client_nonce("");
    out->set_bridge_generation(0);
    out->set_world_loaded(false);
    out->set_fortress_mode(false);
    out->set_paused(false);
    out->set_current_year(0);
    out->set_current_year_tick(0);
    out->set_world_name("");
    out->set_world_folder("");
    out->set_site_id(-1);
    out->set_citizen_count_total(0);
    out->set_citizen_offset(0);
    out->set_complete(false);
}

command_result ReadObservation(color_ostream &,
                               const wire::ReadObservationRequest *in,
                               wire::ReadObservationReply *out)
{
    initialize_observation_reply(out, in->protocol_minor());

    std::string failure_code;
    std::string failure_message;
    if (in->client_nonce().size() < MIN_NONCE_BYTES ||
        in->client_nonce().size() > MAX_NONCE_BYTES)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message("client nonce violates the 16..64 byte policy");
        return CR_OK;
    }
    out->set_client_nonce(in->client_nonce());
    out->set_bridge_generation(BRIDGE_GENERATION);
    if (!validate_protocol(in->protocol_major(), in->protocol_minor(),
                           failure_code, failure_message) ||
        !authenticate(in->bearer_token(), failure_code, failure_message))
    {
        out->set_failure_code(failure_code);
        out->set_failure_message(failure_message);
        return CR_OK;
    }

    const std::uint32_t requested =
        in->has_max_citizens() ? in->max_citizens() : DEFAULT_MAX_CITIZENS;
    if (requested == 0 || requested > HARD_MAX_CITIZENS)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message("max_citizens must be in the range 1..4096");
        return CR_OK;
    }

    const bool world_loaded = Core::getInstance().isWorldLoaded();
    out->set_world_loaded(world_loaded);
    if (!world_loaded)
    {
        out->set_failure_code("WORLD_NOT_LOADED");
        out->set_failure_message("no Dwarf Fortress world is loaded");
        return CR_OK;
    }

    const bool fortress_mode = World::isFortressMode();
    out->set_fortress_mode(fortress_mode);
    if (!fortress_mode)
    {
        out->set_failure_code("NOT_FORTRESS_MODE");
        out->set_failure_message("the loaded world is not in fortress mode");
        return CR_OK;
    }

    const std::uint32_t year_tick = World::ReadCurrentTick();
    const std::int32_t site_id = World::GetCurrentSiteId();
    if (year_tick >= TICKS_PER_YEAR || site_id < 0)
    {
        out->set_failure_code("INTERNAL_FAILURE");
        out->set_failure_message(
            "fortress clock or site identity is outside the canonical domain");
        return CR_OK;
    }

    out->set_paused(World::ReadPauseState());
    out->set_current_year(World::ReadCurrentYear());
    out->set_current_year_tick(year_tick);
    out->set_world_name(
        bounded_utf8_prefix(World::getWorldName(false), MAX_WORLD_NAME_BYTES));
    out->set_world_folder(
        bounded_utf8_prefix(World::ReadWorldFolder(), MAX_WORLD_FOLDER_BYTES));
    out->set_site_id(site_id);

    std::vector<df::unit *> citizens;
    if (!Units::getCitizens(citizens, true, false))
    {
        out->set_failure_code("INTERNAL_FAILURE");
        out->set_failure_message("DFHack could not enumerate fortress citizens");
        return CR_OK;
    }
    citizens.erase(
        std::remove(citizens.begin(), citizens.end(), nullptr), citizens.end());
    if (citizens.size() > std::numeric_limits<std::uint32_t>::max())
    {
        out->set_failure_code("INTERNAL_FAILURE");
        out->set_failure_message("citizen roster exceeds the count domain");
        return CR_OK;
    }
    std::sort(citizens.begin(), citizens.end(),
              [](const df::unit *left, const df::unit *right) {
                  return left->id < right->id;
              });

    const std::size_t total = citizens.size();
    const std::size_t offset = std::min<std::size_t>(in->citizen_offset(), total);
    const std::size_t end = std::min<std::size_t>(
        total, offset + static_cast<std::size_t>(requested));

    out->set_citizen_count_total(static_cast<std::uint32_t>(total));
    out->set_citizen_offset(static_cast<std::uint32_t>(offset));
    out->set_complete(end == total);

    for (std::size_t index = offset; index < end; ++index)
    {
        df::unit *unit = citizens[index];
        if (!Units::isCitizen(unit, false))
        {
            out->set_failure_code("INTERNAL_FAILURE");
            out->set_failure_message(
                "DFHack returned a non-citizen in the strict citizen roster");
            out->clear_citizens();
            out->set_citizen_count_total(0);
            out->set_citizen_offset(0);
            out->set_complete(false);
            return CR_OK;
        }

        auto *record = out->add_citizens();
        record->set_unit_id(unit->id);
        if (!in->has_include_names() || in->include_names())
        {
            const auto *visible_name = Units::getVisibleName(unit);
            const std::string translated_name = visible_name
                ? Translation::translateName(visible_name, false)
                : std::string();
            record->set_name(
                bounded_utf8_prefix(translated_name, MAX_UNIT_NAME_BYTES));
        }
        else
        {
            record->set_name("");
        }
        record->set_race(bounded_utf8_prefix(
            Units::getRaceReadableName(unit), MAX_RACE_NAME_BYTES));
        record->set_profession(
            static_cast<std::int32_t>(Units::getProfession(unit)));
        const df::coord position = Units::getPosition(unit);
        record->set_x(position.x);
        record->set_y(position.y);
        record->set_z(position.z);
        record->set_alive(Units::isAlive(unit));
        record->set_sane(Units::isSane(unit));
        record->set_active(Units::isActive(unit));
        record->set_visible(Units::isVisible(unit));
        record->set_citizen(Units::isCitizen(unit, false));
        record->set_resident(Units::isResident(unit));
        record->set_baby(Units::isBaby(unit));
        record->set_child(Units::isChild(unit));
        record->set_adult(Units::isAdult(unit));
    }

    out->set_accepted(true);
    return CR_OK;
}

void initialize_announcement_reply(wire::ReadAnnouncementsReply *out)
{
    out->set_accepted(false);
    out->set_failure_code("");
    out->set_failure_message("");
    out->set_protocol_major(PROTOCOL_MAJOR);
    out->set_protocol_minor(ANNOUNCEMENT_PROTOCOL_MINOR);
    out->set_client_nonce("");
    out->set_bridge_generation(0);
    out->set_requested_after_report_id(-1);
    out->set_oldest_retained_report_id(-1);
    out->set_latest_retained_report_id(-1);
    out->set_window_latest_report_id(-1);
    out->set_next_after_report_id(-1);
    out->set_history_truncated(false);
    out->set_complete(false);
}

bool is_retained_announcement(const df::report *report)
{
    return report && report->id >= 0 && report->flags.bits.announcement;
}

command_result ReadAnnouncements(color_ostream &,
                                 const wire::ReadAnnouncementsRequest *in,
                                 wire::ReadAnnouncementsReply *out)
{
    initialize_announcement_reply(out);

    std::string failure_code;
    std::string failure_message;
    if (in->client_nonce().size() < MIN_NONCE_BYTES ||
        in->client_nonce().size() > MAX_NONCE_BYTES)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message("client nonce violates the 16..64 byte policy");
        return CR_OK;
    }
    out->set_client_nonce(in->client_nonce());
    out->set_bridge_generation(BRIDGE_GENERATION);
    if (!validate_announcement_protocol(
            in->protocol_major(), in->protocol_minor(),
            failure_code, failure_message) ||
        !authenticate(in->bearer_token(), failure_code, failure_message))
    {
        out->set_failure_code(failure_code);
        out->set_failure_message(failure_message);
        return CR_OK;
    }

    const std::int32_t after =
        in->has_after_report_id() ? in->after_report_id() : -1;
    const std::int32_t through =
        in->has_through_report_id() ? in->through_report_id() : -1;
    const std::uint32_t requested = in->has_max_announcements()
        ? in->max_announcements()
        : DEFAULT_MAX_ANNOUNCEMENTS;
    out->set_requested_after_report_id(after);
    out->set_next_after_report_id(after);
    if (after < -1 || through < -1 ||
        (through >= 0 && through < after) ||
        requested == 0 || requested > HARD_MAX_ANNOUNCEMENTS)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message(
            "announcement cursors or page size violate protocol 1.1 bounds");
        return CR_OK;
    }

    if (!Core::getInstance().isWorldLoaded())
    {
        out->set_failure_code("WORLD_NOT_LOADED");
        out->set_failure_message("no Dwarf Fortress world is loaded");
        return CR_OK;
    }
    if (!World::isFortressMode())
    {
        out->set_failure_code("NOT_FORTRESS_MODE");
        out->set_failure_message("the loaded world is not in fortress mode");
        return CR_OK;
    }
    if (!df::global::world)
    {
        out->set_failure_code("INTERNAL_FAILURE");
        out->set_failure_message("the Dwarf Fortress world global is unavailable");
        return CR_OK;
    }

    std::vector<const df::report *> reports;
    reports.reserve(df::global::world->status.reports.size());
    for (const df::report *report : df::global::world->status.reports)
    {
        if (is_retained_announcement(report))
            reports.push_back(report);
    }
    std::sort(reports.begin(), reports.end(),
              [](const df::report *left, const df::report *right) {
                  return left->id < right->id;
              });
    reports.erase(
        std::unique(reports.begin(), reports.end(),
                    [](const df::report *left, const df::report *right) {
                        return left->id == right->id;
                    }),
        reports.end());

    if (reports.empty())
    {
        if (after >= 0 || through >= 0)
        {
            out->set_failure_code("STALE_ANCHOR");
            out->set_failure_message(
                "the requested announcement cursor is not retained in this world");
            return CR_OK;
        }
        out->set_accepted(true);
        out->set_complete(true);
        return CR_OK;
    }

    const std::int32_t oldest = reports.front()->id;
    const std::int32_t latest = reports.back()->id;
    const std::int32_t high_water = through < 0 ? latest : through;
    out->set_oldest_retained_report_id(oldest);
    out->set_latest_retained_report_id(latest);
    out->set_window_latest_report_id(high_water);

    if (after > latest || high_water > latest)
    {
        out->set_failure_code("STALE_ANCHOR");
        out->set_failure_message(
            "the requested announcement cursor exceeds the retained high-water mark");
        return CR_OK;
    }

    const bool history_truncated = after >= 0 &&
        after < oldest - 1;
    out->set_history_truncated(history_truncated);

    std::size_t returned = 0;
    bool more_in_window = false;
    std::int32_t next_after = after;
    for (const df::report *report : reports)
    {
        if (report->id <= after || report->id > high_water)
            continue;
        if (returned >= requested)
        {
            more_in_window = true;
            break;
        }
        if (report->year < 0 || report->time < 0 ||
            static_cast<std::uint32_t>(report->time) >= TICKS_PER_YEAR ||
            report->repeat_count < 0)
        {
            out->clear_announcements();
            out->set_next_after_report_id(after);
            out->set_complete(false);
            out->set_failure_code("INTERNAL_FAILURE");
            out->set_failure_message(
                "a retained announcement is outside the canonical numeric domain");
            return CR_OK;
        }

        auto *record = out->add_announcements();
        record->set_report_id(report->id);
        record->set_announcement_type(
            static_cast<std::int32_t>(report->type));
        record->set_text(bounded_utf8_prefix(
            DF2UTF(report->text), MAX_ANNOUNCEMENT_TEXT_BYTES));
        record->set_year(static_cast<std::uint32_t>(report->year));
        record->set_year_tick(static_cast<std::uint32_t>(report->time));
        const bool has_position = report->pos.x >= 0 &&
            report->pos.y >= 0 && report->pos.z >= 0;
        record->set_has_position(has_position);
        record->set_x(has_position ? report->pos.x : 0);
        record->set_y(has_position ? report->pos.y : 0);
        record->set_z(has_position ? report->pos.z : 0);
        record->set_repeat_count(
            static_cast<std::uint32_t>(report->repeat_count));
        record->set_continuation(report->flags.bits.continuation);
        record->set_unconscious(report->flags.bits.unconscious);
        record->set_announcement(report->flags.bits.announcement);
        next_after = report->id;
        ++returned;
    }

    out->set_next_after_report_id(next_after);
    out->set_complete(!more_in_window);
    out->set_accepted(true);
    return CR_OK;
}

command_result bridge_status(color_ostream &out,
                             std::vector<std::string> &)
{
    const char *configured = std::getenv("DFMCP_BRIDGE_TOKEN");
    const std::size_t configured_size =
        configured ? std::string_view(configured).size() : 0;
    const bool token_configured = configured_size >= MIN_TOKEN_BYTES &&
        configured_size <= MAX_TOKEN_BYTES;
    out.print("dfmcp_bridge {} protocols 1.0, 1.1\n", BRIDGE_VERSION);
    out.print("token policy satisfied: {}\n",
              token_configured ? "yes" : "no");
    out.print("world loaded: {}\n",
              Core::getInstance().isWorldLoaded() ? "yes" : "no");
    out.print(
        "RPC methods: Handshake, ReadObservation, ReadAnnouncements\n");
    return token_configured ? CR_OK : CR_FAILURE;
}

} // namespace

DFhackCExport command_result plugin_init(color_ostream &,
                                         std::vector<PluginCommand> &commands)
{
    commands.emplace_back(
        "dfmcp-bridge-status",
        "Report the read-only dfmcp bridge protocol and token posture",
        bridge_status,
        false);
    return CR_OK;
}

DFhackCExport RPCService *plugin_rpcconnect(color_ostream &)
{
    auto *service = new RPCService();
    service->addFunction("Handshake", Handshake, 0);
    service->addFunction("ReadObservation", ReadObservation, 0);
    service->addFunction("ReadAnnouncements", ReadAnnouncements, 0);
    return service;
}

DFhackCExport command_result plugin_shutdown(color_ostream &)
{
    return CR_OK;
}
