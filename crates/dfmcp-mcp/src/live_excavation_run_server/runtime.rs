//! Inherited Asupersync-owned blocking work; no detached cancellation watcher.
use super::*;
use asupersync::Cx;
use dfmcp_adapter::excavation_run::rpc::ExcavationCancellation;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

const NAMES: [&str; 4] = ["DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18",
    "DFMCP_EXCAVATION_RUN_TOKEN", "DFMCP_EXCAVATION_RUN_ENDPOINT", "DFMCP_EXCAVATION_RUN_ALLOW_CLOCK"];

pub(super) fn isolated(opt: Option<&str>, names: impl IntoIterator<Item = String>, admitted: bool) -> Result<()> {
    if opt != Some("1") || admitted { return Err(denied()); }
    for (index, name) in names.into_iter().enumerate() {
        if index >= 1024 || name.len() > 512
            || (name.starts_with("DFMCP_") && !NAMES.contains(&name.as_str())) { return Err(denied()); }
    }
    Ok(())
}
pub(super) fn environment() -> Result<()> {
    // Offline paths check names and this exact opt-in only. They never consult
    // token or endpoint configuration or construct a native connection.
    isolated(std::env::var(NAMES[0]).ok().as_deref(),
        std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned()),
        crate::admission::current_admission_provenance().is_some())
}
pub(super) fn clock_enabled() -> Result<()> {
    if std::env::var(NAMES[3]).ok().as_deref() != Some("1") { return Err(denied()); }
    Ok(())
}
pub(super) struct Control {
    pub started: Instant,
    parent: Cx,
    worker: Cx,
    abandoned: Arc<AtomicBool>,
    cancellation: ExcavationCancellation,
}
impl Control {
    fn runtime_checkpoint(&self) -> Result<()> {
        if self.abandoned.load(Ordering::Acquire) || self.parent.checkpoint().is_err()
            || self.worker.checkpoint().is_err() || self.cancellation.is_cancelled()
        {
            self.cancellation.cancel();
            return Err(error(ErrorCode::CancellationRequested, "excavation request cancelled"));
        }
        if self.parent.io().is_none() || self.worker.io().is_none() {
            self.cancellation.cancel(); return Err(denied());
        }
        Ok(())
    }
}
impl ExcavationSessionGuard for Control {
    fn checkpoint(&self) -> Result<()> { self.runtime_checkpoint()?; environment() }
    fn allow_start(&self) -> Result<()> { self.checkpoint()?; clock_enabled() }
    fn cancellation(&self) -> ExcavationCancellation { self.cancellation.clone() }
}
struct CancelOnDrop { abandoned: Arc<AtomicBool>, cancellation: ExcavationCancellation }
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::Release);
        self.cancellation.cancel();
    }
}
pub(super) async fn owned<F>(op: &'static str, f: F) -> String
where F: FnOnce(Control) -> String + Send + 'static,
{
    let Some(cx) = Cx::current() else { return unbound(op, &denied()); };
    if cx.checkpoint().is_err() || cx.io().is_none() { return unbound(op, &denied()); }
    let started = Instant::now();
    let abandoned = Arc::new(AtomicBool::new(false));
    let cancellation = ExcavationCancellation::default();
    let _cancel = CancelOnDrop { abandoned: abandoned.clone(), cancellation: cancellation.clone() };
    let parent = cx.clone();
    let worker_cancel = cancellation.clone();
    let task = cx.spawn_blocking(move |worker| {
        let _ambient = Cx::set_current(Some(worker.clone()));
        let control = Control { started, parent, worker, abandoned, cancellation: worker_cancel };
        if let Err(cause) = control.runtime_checkpoint() { return unbound(op, &cause); }
        f(control)
    });
    let mut task = match task { Ok(task) => task, Err(_) => return unbound(op, &denied()) };
    let mut joined = pin!(task.join(&cx));
    let result = poll_fn(|context| {
        // Cancellation wakes the join; signal the existing bounded socket's
        // handle before polling it. Dropping this future signals it too.
        if cx.checkpoint().is_err() || cx.io().is_none() { cancellation.cancel(); }
        joined.as_mut().poll(context)
    }).await;
    match result {
        Ok(value) => value,
        Err(_) => unbound(op, &error(ErrorCode::CancellationIncomplete,
            "no acknowledged excavation result; recover the original journal, never repeat commit")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    #[test]
    fn runtime_owned_work_is_joined_with_inherited_context() -> std::result::Result<(),Box<dyn Error>> {
        crate::run_with_runtime_cx(|parent| async move {
            let caller=std::thread::current().id();
            let parent_region=parent.region_id();
            let value=owned("fortress.query",move|control|{
                control.runtime_checkpoint().unwrap();
                assert_ne!(std::thread::current().id(),caller);
                assert_eq!(control.parent.region_id(),parent_region);
                assert_eq!(Cx::current().unwrap().task_id(),control.worker.task_id());
                "joined".into()
            }).await;
            assert_eq!(value,"joined");
        })?;
        Ok(())
    }
    #[test]
    fn inherited_restrictions_never_create_fallback_work() -> std::result::Result<(),Box<dyn Error>> {
        crate::run_with_runtime_cx(|parent| async move {
            let _restricted=parent.restrict::<asupersync::cx::cap::None>().set_current_restricted();
            let result=owned("fortress.commit",|_|panic!("restricted callback executed")).await;
            let result:Value=serde_json::from_str(&result).unwrap();
            assert_eq!(result["result"]["ok"],false);
            assert_eq!(result["agent_turn"]["active_work"]["inventory_verified"],false);
        })?;
        Ok(())
    }
    #[test]
    fn cancelled_parent_cannot_enter_the_blocking_effect_shell() -> std::result::Result<(),Box<dyn Error>> {
        crate::run_with_runtime_cx(|parent| async move {
            parent.cancel_with(asupersync::types::CancelKind::User,Some("test cancellation"));
            let result=owned("fortress.commit",|_|panic!("cancelled callback executed")).await;
            assert_eq!(serde_json::from_str::<Value>(&result).unwrap()["result"]["ok"],false);
        })?;
        Ok(())
    }
    #[test]
    fn abandoned_request_signals_the_native_socket_cancellation_handle() {
        let abandoned=Arc::new(AtomicBool::new(false));
        let cancellation=ExcavationCancellation::default();
        {let _guard=CancelOnDrop{abandoned:abandoned.clone(),cancellation:cancellation.clone()};}
        assert!(abandoned.load(Ordering::Acquire));
        assert!(cancellation.is_cancelled());
    }
}
