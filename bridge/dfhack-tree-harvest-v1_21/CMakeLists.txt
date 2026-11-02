# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpTreeHarvestV1_21.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpTreeHarvestV1_21.proto" COPYONLY)
# Isolated development protocol. No production runner or admission is added.
dfhack_plugin(dfmcp_tree_harvest_v1_21 dfmcp_tree_harvest_v1_21.cpp PROTOBUFS DfmcpTreeHarvestV1_21
    LINK_LIBRARIES protobuf)
