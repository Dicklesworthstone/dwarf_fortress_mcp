# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpWorkforceV1_17.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpWorkforceV1_17.proto" COPYONLY)
# Development only: add to an exact matching DFHack tree; retain ../common.
dfhack_plugin(dfmcp_workforce_v1_17 dfmcp_workforce_v1_17.cpp PROTOBUFS DfmcpWorkforceV1_17
    LINK_LIBRARIES protobuf)
