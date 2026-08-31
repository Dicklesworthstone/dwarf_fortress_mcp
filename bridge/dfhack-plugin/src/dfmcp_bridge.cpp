#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <string>
#include <vector>

#include "Core.h"
#include "Console.h"
#include "DfmcpBridge.pb.h"
#include "Export.h"
#include "PluginManager.h"
#include "RemoteServer.h"
#include "VersionInfo.h"
#include "modules/Translation.h"
#include "modules/Units.h"
#include "modules/World.h"

#include "df/unit.h"

using namespace DFHack;
namespace wire = dfmcp::bridge::v1;

DFHACK_PLUGIN("dfmcp_bridge");

namespace {

constexpr std::uint32_t PROTOCOL_MAJOR = 1;
constexpr std::uint32_t PROTOCOL_MINOR = 0;
constexpr const char *BRIDGE_VERSION = "0.1.0";
constexpr std::size_t MIN_TOKEN_BYTES = 32;
constexpr std::size_t MAX_TOKEN_BYTES = 256;
constexpr std::size_t MIN_NONCE_BYTES = 16;
constexpr std::size_t MAX_NONCE_BYTES = 64;
constexpr std::size_t MAX_CLIENT_NAME_BYTES = 128;
constexpr std::size_t MAX_CLIENT_VERSION_BYTES = 64;
constexpr std::uint32_t DEFAULT_MAX_CITIZENS = 256;
constexpr std::uint32_t HARD_MAX_CITIZENS = 4096;
constexpr std::size_t MAX_UNIT_NAME_BYTES = 256;
constexpr std::size_t MAX_RACE_NAME_BYTES = 128;
constexpr std::size_t MAX_WORLD_NAME_BYTES = 256;
constexpr std::size_t MAX_WORLD_FOLDER_BYTES = 512;

const std::uint64_t BRIDGE_GENERATION =
    static_cast<std::uint64_t>(
        std::chrono::steady_clock::now().time_since_epoch().count());

std::string bounded_utf8_prefix(const std::string &value, std::size_t max_bytes)
{
    if (value.size() <= max_bytes)
        return value;

    std::size_t offset = 0;
    std::size_t last_complete = 0;
    while (offset < value.size() && offset < max_bytes)
    {
        const auto lead = static_cast<unsigned char>(value[offset]);
        std::size_t width = 0;
        if ((lead & 0x80U) == 0)
            width = 1;
        else if ((lead & 0xE0U) == 0xC0U)
            width = 2;
        else if ((lead & 0xF0U) == 0xE0U)
            width = 3;
        else if ((lead & 0xF8U) == 0xF0U)
            width = 4;
        else
            break;

        if (offset + width > value.size() || offset + width > max_bytes)
            break;

        bool valid = true;
        for (std::size_t index = 1; index < width; ++index)
        {
            const auto continuation =
                static_cast<unsigned char>(value[offset + index]);
            if ((continuation & 0xC0U) != 0x80U)
            {
                valid = false;
                break;
            }
        }
        if (!valid)
            break;

        offset += width;
        last_complete = offset;
    }
    return value.substr(0, last_complete);
}

bool constant_time_equal(const std::string &left, const std::string &right)
{
    const std::size_t maximum = std::max(left.size(), right.size());
    std::size_t difference = left.size() ^ right.size();
    for (std::size_t index = 0; index < maximum; ++index)
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

    const std::string expected(configured);
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

bool validate_protocol(std::uint32_t major, std::uint32_t minor,
                       std::string &failure_code,
                       std::string &failure_message)
{
    if (major != PROTOCOL_MAJOR || minor != PROTOCOL_MINOR)
    {
        failure_code = "PROTOCOL_MISMATCH";
        failure_message = "bridge protocol must be exactly 1.0";
        return false;
    }
    return true;
}

std::string df_version()
{
    const auto *version_info = Core::getInstance().vinfo;
    return version_info ? version_info->getVersion() : std::string("unknown");
}

void initialize_handshake_reply(wire::HandshakeReply *out,
                                const std::string &client_nonce)
{
    out->set_accepted(false);
    out->set_failure_code("");
    out->set_failure_message("");
    out->set_protocol_major(PROTOCOL_MAJOR);
    out->set_protocol_minor(PROTOCOL_MINOR);
    out->set_bridge_version(BRIDGE_VERSION);
    out->set_dfhack_version(Version::dfhack_version());
    out->set_df_version(df_version());
    const bool world_loaded = Core::getInstance().isWorldLoaded();
    out->set_world_loaded(world_loaded);
    out->set_fortress_mode(world_loaded && World::isFortressMode());
    out->set_client_nonce(client_nonce);
    out->set_bridge_generation(BRIDGE_GENERATION);
    out->add_supported_methods("Handshake");
    out->add_supported_methods("ReadObservation");
}

command_result Handshake(color_ostream &, const wire::HandshakeRequest *in,
                         wire::HandshakeReply *out)
{
    initialize_handshake_reply(out, in->client_nonce());

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
    if (!validate_protocol(in->protocol_major(), in->protocol_minor(),
                           failure_code, failure_message) ||
        !authenticate(in->bearer_token(), failure_code, failure_message))
    {
        out->set_failure_code(failure_code);
        out->set_failure_message(failure_message);
        return CR_OK;
    }

    out->set_accepted(true);
    return CR_OK;
}

void initialize_observation_reply(wire::ReadObservationReply *out,
                                  const std::string &client_nonce)
{
    out->set_accepted(false);
    out->set_failure_code("");
    out->set_failure_message("");
    out->set_protocol_major(PROTOCOL_MAJOR);
    out->set_protocol_minor(PROTOCOL_MINOR);
    out->set_client_nonce(client_nonce);
    out->set_bridge_generation(BRIDGE_GENERATION);
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
    initialize_observation_reply(out, in->client_nonce());

    std::string failure_code;
    std::string failure_message;
    if (in->client_nonce().size() < MIN_NONCE_BYTES ||
        in->client_nonce().size() > MAX_NONCE_BYTES)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message("client nonce violates the 16..64 byte policy");
        return CR_OK;
    }
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
    if (requested > HARD_MAX_CITIZENS)
    {
        out->set_failure_code("INVALID_BOUND");
        out->set_failure_message("max_citizens exceeds the hard limit of 4096");
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

    out->set_paused(World::ReadPauseState());
    out->set_current_year(World::ReadCurrentYear());
    out->set_current_year_tick(World::ReadCurrentTick());
    out->set_world_name(
        bounded_utf8_prefix(World::getWorldName(false), MAX_WORLD_NAME_BYTES));
    out->set_world_folder(
        bounded_utf8_prefix(World::ReadWorldFolder(), MAX_WORLD_FOLDER_BYTES));
    out->set_site_id(World::GetCurrentSiteId());

    std::vector<df::unit *> citizens;
    if (!Units::getCitizens(citizens, false, false))
    {
        out->set_failure_code("INTERNAL_FAILURE");
        out->set_failure_message("DFHack could not enumerate fortress citizens");
        return CR_OK;
    }
    citizens.erase(
        std::remove(citizens.begin(), citizens.end(), nullptr), citizens.end());
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
        auto *record = out->add_citizens();
        record->set_unit_id(unit->id);
        if (!in->has_include_names() || in->include_names())
        {
            record->set_name(bounded_utf8_prefix(
                Translation::translateName(&unit->name, false),
                MAX_UNIT_NAME_BYTES));
        }
        else
        {
            record->set_name("");
        }
        record->set_race(bounded_utf8_prefix(
            Units::getRaceReadableName(unit), MAX_RACE_NAME_BYTES));
        record->set_profession(static_cast<std::int32_t>(unit->profession));
        const df::coord position = Units::getPosition(unit);
        record->set_x(position.x);
        record->set_y(position.y);
        record->set_z(position.z);
        record->set_alive(Units::isAlive(unit));
        record->set_sane(Units::isSane(unit));
        record->set_active(Units::isActive(unit));
        record->set_visible(Units::isVisible(unit));
        record->set_citizen(Units::isCitizen(unit, false));
        record->set_resident(Units::isResident(unit, false));
        record->set_baby(Units::isBaby(unit));
        record->set_child(Units::isChild(unit));
        record->set_adult(Units::isAdult(unit));
        record->set_military(unit->military.squad_id >= 0);
    }

    out->set_accepted(true);
    return CR_OK;
}

command_result bridge_status(color_ostream &out,
                             std::vector<std::string> &)
{
    const char *configured = std::getenv("DFMCP_BRIDGE_TOKEN");
    const bool token_configured = configured &&
        std::string(configured).size() >= MIN_TOKEN_BYTES &&
        std::string(configured).size() <= MAX_TOKEN_BYTES;
    out.print("dfmcp_bridge %s protocol %u.%u\n", BRIDGE_VERSION,
              PROTOCOL_MAJOR, PROTOCOL_MINOR);
    out.print("token policy satisfied: %s\n",
              token_configured ? "yes" : "no");
    out.print("world loaded: %s\n",
              Core::getInstance().isWorldLoaded() ? "yes" : "no");
    out.print("RPC methods: Handshake, ReadObservation\n");
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
    return service;
}

DFhackCExport command_result plugin_shutdown(color_ostream &)
{
    return CR_OK;
}
