#![forbid(unsafe_code)]
//! Public-API acquisition tests using the same injected protocol/storage fixture
//! as the existing semantic session unit tests. Link the ordinary production
//! library so no private implementation or filesystem overlay is required.

pub use dfmcp_adapter::*;

#[allow(dead_code, unused_imports)]
mod semantic_workforce_acquisition {
    include!("../src/semantic_workforce/tests/fixture.rs");

    mod evidence_owner {
        include!("../src/semantic_workforce/tests/evidence_owner.rs");
    }

    mod goal_monitor {
        include!("../src/semantic_workforce/tests/goal_monitor.rs");
    }
}
