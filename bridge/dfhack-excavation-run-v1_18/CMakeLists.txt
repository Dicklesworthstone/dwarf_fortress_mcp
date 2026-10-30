# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpExcavationRunV1_18.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpExcavationRunV1_18.proto" COPYONLY)
# Isolated development protocol. Never added to the production runner map.
dfhack_plugin(dfmcp_excavation_run_v1_18 dfmcp_excavation_run_v1_18.cpp
    PROTOBUFS DfmcpExcavationRunV1_18
    LINK_LIBRARIES protobuf)
