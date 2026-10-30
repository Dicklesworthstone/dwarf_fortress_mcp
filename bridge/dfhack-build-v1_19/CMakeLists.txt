# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpBuildV1_19.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpBuildV1_19.proto" COPYONLY)
# Isolated development protocol. No production runner or admission is added.
dfhack_plugin(dfmcp_build_v1_19 dfmcp_build_v1_19.cpp PROTOBUFS DfmcpBuildV1_19
    LINK_LIBRARIES protobuf)
