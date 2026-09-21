//! Recheck runtime/operator authority at each native edge, including AFTER sync.
use dfmcp_adapter::workforce_control::{AssignmentEffect, AssignmentPlan, WorkforceCapture};
use dfmcp_adapter::workforce_control::rpc::{WorkforceManifest, WorkforceSource};
use dfmcp_core::{OperationContext, Result};
use std::net::SocketAddr;

pub(super) struct CheckedSource<N, F> {
    pub source: N,
    pub check: F,
}
impl<N: WorkforceSource, F: FnMut(bool) -> Result<()>> WorkforceSource for CheckedSource<N, F> {
    fn manifest(&self) -> &WorkforceManifest { self.source.manifest() }
    fn endpoint(&self) -> Option<SocketAddr> { self.source.endpoint() }
    fn observe(&mut self, ids: &[u32], c: &OperationContext) -> Result<WorkforceCapture> {
        (self.check)(false)?;
        self.source.observe(ids, c)
    }
    fn prepare(&mut self, p: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        (self.check)(true)?;
        self.source.prepare(p, c)
    }
    fn commit(&mut self, p: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        (self.check)(true)?;
        self.source.commit(p, c)
    }
    fn query(&mut self, p: &AssignmentPlan, c: &OperationContext) -> Result<Option<AssignmentEffect>> {
        (self.check)(false)?;
        self.source.query(p, c)
    }
    fn cancel(&mut self, p: &AssignmentPlan, c: &OperationContext) -> Result<AssignmentEffect> {
        (self.check)(true)?;
        self.source.cancel(p, c)
    }
}
