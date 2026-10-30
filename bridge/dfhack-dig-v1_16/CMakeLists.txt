# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpDigV1_16.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpDigV1_16.proto" COPYONLY)
# Copy alongside common/ into a matching DFHack plugins source checkout.
dfhack_plugin(dfmcp_dig_v1_16 dfmcp_dig_v1_16.cpp PROTOBUFS DfmcpDigV1_16
    LINK_LIBRARIES protobuf)
