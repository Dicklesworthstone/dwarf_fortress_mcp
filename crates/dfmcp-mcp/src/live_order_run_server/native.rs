//! Runtime/environment checks immediately before each native effect boundary.
use dfmcp_adapter::order_run::rpc::{OrderRunManifest, OrderRunSource};
use dfmcp_adapter::order_run::{FortressIdentity, OrderCapture, OrderRunPlan, OrderRunRecord};
use dfmcp_core::{OperationContext, Result};
use std::net::SocketAddr;
use std::time::Duration;

pub struct CheckedSource<N> {
    pub inner: N,
    pub check: fn(bool, &FortressIdentity) -> Result<()>,
}
impl<N: OrderRunSource> OrderRunSource for CheckedSource<N> {
    fn manifest(&self) -> &OrderRunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn fortress(&self) -> &FortressIdentity {
        self.inner.fortress()
    }
    fn fence(&mut self) {
        self.inner.fence();
    }
    fn observe(&mut self, id: u32, c: &OperationContext, t: Duration) -> Result<OrderCapture> {
        (self.check)(false, self.inner.fortress())?;
        self.inner.observe(id, c, t)
    }
    fn prepare(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        (self.check)(true, self.inner.fortress())?;
        self.inner.prepare(p, c, t)
    }
    fn commit(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        (self.check)(true, self.inner.fortress())?;
        self.inner.commit(p, c, t)
    }
    fn query(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        (self.check)(false, self.inner.fortress())?;
        self.inner.query(p, c, t)
    }
    fn cancel(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        (self.check)(true, self.inner.fortress())?;
        self.inner.cancel(p, c, t)
    }
}
