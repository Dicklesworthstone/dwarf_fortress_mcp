# DFHACK_PLUGIN expects proto/<stem>.proto and adds the extension itself.
file(MAKE_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/proto")
configure_file("${CMAKE_CURRENT_SOURCE_DIR}/DfmcpWorkshopV1_20.proto"
    "${CMAKE_CURRENT_SOURCE_DIR}/proto/DfmcpWorkshopV1_20.proto" COPYONLY)
# Isolated development protocol. No production runner or admission is added.
dfhack_plugin(dfmcp_workshop_v1_20 dfmcp_workshop_v1_20.cpp PROTOBUFS DfmcpWorkshopV1_20
    LINK_LIBRARIES protobuf)
