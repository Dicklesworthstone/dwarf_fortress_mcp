//! Preserve the entire native shelter selection in an ordinary watch request.
//! Producing this request registers nothing and never imports positional claims.
use super::*;

const MAX_ARGUMENT_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Spec {
    key: String,
    label: Option<String>,
    deadline_tick: u64,
    poll_interval_ticks: Option<u64>,
    stable_observations: Option<u32>,
}

impl Spec {
    fn policy(&self, context: &OperationContext) -> Result<(u64, u32, &str)> {
        let label = self.label.as_deref().unwrap_or(self.key.as_str());
        for (value, maximum) in [(self.key.as_str(), 64), (label, 256)] {
            if value.is_empty() || value.len() > maximum || value.contains('\0') {
                return Err(invalid("shelter watch key/label exceeds its nonempty byte bound"));
            }
        }
        let interval = self.poll_interval_ticks.unwrap_or(1);
        let samples = self.stable_observations.unwrap_or(2);
        if !(1..=1_000_000).contains(&interval) || !(1..=64).contains(&samples) {
            return Err(invalid("shelter watch cadence must be 1..1000000 and samples 1..64"));
        }
        let horizon = self.deadline_tick.checked_sub(context.anchor.tick.0)
            .filter(|left| *left > 0 && *left <= context.budget.max_game_ticks)
            .ok_or_else(|| invalid("shelter watch deadline must be future and within the session horizon"))?;
        let span = u64::from(samples - 1).checked_mul(interval)
            .ok_or_else(|| budget("shelter watch sample horizon overflow"))?;
        if span > horizon {
            return Err(invalid("shelter watch deadline cannot fit even the earliest requested samples"));
        }
        Ok((interval, samples, label))
    }

    pub(super) fn build(&self, selection: &ShelterSelection, context: &OperationContext) -> Result<Value> {
        let (interval, samples, label) = self.policy(context)?;
        // Selection is already normalized and validated. Use ALL original
        // targets, including missing/outside citizens, never the current page.
        let condition = json!({"op":"shelter_set",
            "citizens":selection.citizens().iter().map(|target| json!({
                "entity_id":target.entity_id.to_string(),"generation":target.generation
            })).collect::<Vec<_>>(),
            "shelters":selection.shelters().iter().map(|area| json!({
                "min":coordinates(area.min),"max":coordinates(area.max)
            })).collect::<Vec<_>>()});
        let arguments = json!({"session_id":context.session_id.to_string(),"query":{
            "schema":"dfmcp.query/1","expected_anchor":anchor_json(context.anchor),
            "query":{"kind":"watch","key":self.key,"label":label,"condition":condition,
                "deadline_tick":self.deadline_tick,"poll_interval_ticks":interval,
                "stable_observations":samples}}});
        let arguments_json = arguments.to_string();
        if arguments_json.len() > MAX_ARGUMENT_BYTES {
            return Err(budget("complete shelter watch arguments exceed 16 KiB"));
        }
        // Fixed target/region/string bounds keep the generated envelope within
        // the watch engine's unchanged 1024-node/32768-accounted-byte limits.
        // The full request is a scalar: no output layer may shorten its roster.
        Ok(json!({"tool":"fortress.query","arguments_json":arguments_json,
            "request_digest":Digest32::of_bytes(arguments_json.as_bytes()).to_string(),
            "selection_digest":selection.digest().to_string(),"selected_citizens":selection.citizens().len(),
            "watch_registered":false,"requires_explicit_registration":true,
            "prospective_samples_only":true,"control_authority_granted":false,
            "note":"Submit these complete arguments in the same session at this anchor. The digest is not a watch handle, and prior shelter-status reads supply no stability samples."}))
    }
}

#[cfg(test)]
#[path = "shelter_watch_handoff_tests.rs"]
mod tests;
