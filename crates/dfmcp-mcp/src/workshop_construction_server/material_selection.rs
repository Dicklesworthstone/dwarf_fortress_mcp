//! Material acquisition before the immutable workshop intent is written.
use super::{
    CONNECT_BYTES, ErrorCode, PlanInput, Runtime, Work, WorkshopSource, anchor,
    denied, digest, error, invalid, material,
};
use dfmcp_core::Result;

impl Runtime {
    pub(super) fn observe_material(&mut self, work: &mut Work) -> Result<()> {
        let selection = match &mut self.tree_material {
            Some(donor) => donor.refresh(&self.config, work)?,
            None => self.config.selection.ok_or_else(|| denied("recovery cannot discover a new workshop material"))?,
        };
        let mut source = self
            .config
            .source(&work.reserve(CONNECT_BYTES)?, self.mode)?;
        let current = work.reserve(CONNECT_BYTES)?;
        let capture = source.observe(
            selection,
            &current,
            std::time::Duration::from_millis(current.budget.max_wall_millis),
        )?;
        self.config.capture(&capture)?;
        self.authority_tick = self.authority_tick.max(capture.tick());
        work.advance_authorization_tick(self.authority_tick);
        if let Some(donor) = &mut self.tree_material {
            donor.check_capture_clock(&capture)?;
            donor.verify(work)?;
        }
        if capture.tick() < self.anchor.map_or(0, |a| a.tick.get()) {
            return Err(error(
                ErrorCode::StaleAnchor,
                "workshop observation clock regressed",
            ));
        }
        self.anchor = Some(anchor(&capture));
        self.guard.selection = Some(capture.selection());
        self.observation = Some(capture);
        Ok(())
    }

    pub(super) fn select_material(&mut self, request: &PlanInput, work: &mut Work) -> Result<()> {
        if self.config.tree_material.is_some() {
            if request.kind != "carpenters_from_tree_supply" {
                return Err(invalid("Tree material policy requires kind=carpenters_from_tree_supply"));
            }
            let donor = self.tree_material.as_mut().ok_or_else(|| denied("the exact Tree material owner is unavailable"))?;
            let (selection, handle) = donor.select(
                &self.config,
                &request.expected_observation,
                &request.material,
                work,
            )?;
            let mut source = self.config.source(&work.reserve(CONNECT_BYTES)?, self.mode)?;
            let current = work.reserve(CONNECT_BYTES)?;
            let capture = source.observe(
                selection,
                &current,
                std::time::Duration::from_millis(current.budget.max_wall_millis),
            )?;
            self.config.capture(&capture)?;
            self.authority_tick = self.authority_tick.max(capture.tick());
            work.advance_authorization_tick(self.authority_tick);
            donor.qualify(handle, &capture, work)?;
            donor.verify(work)?;
            if capture.tick() < self.anchor.map_or(0, |anchor| anchor.tick.get()) {
                return Err(error(ErrorCode::StaleAnchor, "workshop observation clock regressed"));
            }
            self.guard.selection = Some(selection);
            self.anchor = Some(anchor(&capture));
            self.observation = Some(capture);
        } else {
            let before = self.observation.as_ref().ok_or_else(|| invalid("observe the original workshop offer first"))?;
            if request.kind != "carpenters"
                || digest(&request.expected_observation)? != before.witness()
                || request.material != material(before)
            {
                return Err(error(ErrorCode::StaleAnchor, "workshop request must select the exact observed Carpenter/log offer"));
            }
        }
        Ok(())
    }
}
