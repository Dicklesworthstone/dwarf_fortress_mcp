//! Semantic codec profiles, fixed by the authenticated runtime, never inferred
//! from payload size or selected by an agent query.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OperationsProfile {
    #[default]
    V1_3,
    PagedV1_4,
}

impl OperationsProfile {
    pub const fn maximum_items(self) -> usize {
        match self { Self::V1_3 => 32_768, Self::PagedV1_4 => 65_536 }
    }
    pub const fn maximum_bytes(self) -> usize {
        match self { Self::V1_3 => 2 * 1024 * 1024, Self::PagedV1_4 => 16 * 1024 * 1024 }
    }
    pub const fn magic(self) -> &'static [u8; 8] {
        match self { Self::V1_3 => b"DFMO1300", Self::PagedV1_4 => b"DFMO1400" }
    }
    pub const fn source_domain(self) -> &'static [u8] {
        match self { Self::V1_3 => b"dfmcp-operations-source-1.3\0", Self::PagedV1_4 => b"dfmcp-operations-source-1.4\0" }
    }
    pub const fn fact_prefix(self) -> &'static str {
        match self { Self::V1_3 => "operations/1.3.", Self::PagedV1_4 => "operations/1.4." }
    }
    pub const fn protocol(self) -> &'static str {
        match self { Self::V1_3 => "1.3", Self::PagedV1_4 => "1.4" }
    }
}
