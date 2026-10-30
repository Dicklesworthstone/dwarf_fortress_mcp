# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpRunV1_13.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpRunV1_13.proto" COPYONLY)
# Add this directory to an exact, matching DFHack source build for development.
dfhack_plugin(dfmcp_run_v1_13 dfmcp_run_v1_13.cpp PROTOBUFS DfmcpRunV1_13
    LINK_LIBRARIES protobuf)
