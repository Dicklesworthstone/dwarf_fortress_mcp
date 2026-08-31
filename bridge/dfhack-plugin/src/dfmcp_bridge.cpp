#include <iostream>

// Compile-only placeholder for the future DFHack plugin. This translation unit
// intentionally does not open a socket, report game versions, read state, or mutate
// Dwarf Fortress. It is not built with DFHack's plugin macros or linked to DFHack.
// Returning failure prevents a manually copied standalone library from impersonating
// a successfully initialized bridge.

extern "C" {

int plugin_init(void*, void*) {
    std::cerr
        << "[dfmcp_bridge] unavailable: genuine DFHack plugin integration, "
           "handshake, and canonical codecs are not implemented"
        << std::endl;
    return -1;
}

int plugin_shutdown(void*) {
    return 0;
}

int plugin_onupdate() {
    return 0;
}

}
