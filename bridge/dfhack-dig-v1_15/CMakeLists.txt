# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpDigV1_15.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpDigV1_15.proto" COPYONLY)
dfhack_plugin(dfmcp_dig_v1_15 dfmcp_dig_v1_15.cpp
    PROTOBUFS DfmcpDigV1_15
    LINK_LIBRARIES protobuf)
