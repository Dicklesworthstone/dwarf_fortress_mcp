//! Closed, bounded request bodies; paths, credentials and native methods are absent.
use super::*;
use serde::de::DeserializeOwned;
use serde::Deserialize;

pub(super) const MAX_REQUEST_BYTES: usize = 2048;
pub(super) const MAX_PAGE: usize = 4;

pub(super) fn parse<T: DeserializeOwned>(raw: &str) -> Result<T> {
    if raw.len() > MAX_REQUEST_BYTES { return Err(invalid()); }
    serde_json::from_str(raw).map_err(|_| invalid())
}
pub(super) fn key(raw: &str) -> Result<()> {
    if raw.is_empty() || raw.len() > 128
        || !raw.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    { return Err(invalid()); }
    Ok(())
}
pub(super) fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(invalid());
    }
    let mut bytes = [0; 32];
    for (i, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| invalid())?;
        bytes[i] = u8::from_str_radix(text, 16).map_err(|_| invalid())?;
    }
    Ok(Digest32::from_bytes(bytes))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanRequest {
    pub key: String,
    pub observation_witness: String,
    pub game_ticks: u32,
    pub wall_millis: u32,
    pub samples: Option<u32>,
    pub stable_ticks: Option<u32>,
    pub interval_ticks: Option<u32>,
    pub max_gap_ticks: Option<u32>,
}
impl PlanRequest {
    pub fn command(self) -> Result<ExcavationCommand> {
        key(&self.key)?;
        let spec = ExcavationRunSpec::new(RunSpec::new(self.game_ticks, self.wall_millis)?,
            self.samples.unwrap_or(2), self.stable_ticks.unwrap_or(10),
            self.interval_ticks.unwrap_or(1), self.max_gap_ticks.unwrap_or(1200))?;
        Ok(ExcavationCommand::Plan { key: self.key, witness: digest(&self.observation_witness)?, spec })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity { pub key: String, pub plan_digest: String }
impl Identity {
    pub fn checked(self) -> Result<(String, Digest32)> {
        key(&self.key)?;
        Ok((self.key, digest(&self.plan_digest)?))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommitRequest { pub key: String, pub plan_digest: String, pub confirm: bool }
impl CommitRequest {
    pub fn command(self) -> Result<ExcavationCommand> {
        key(&self.key)?;
        Ok(ExcavationCommand::Commit { key: self.key, digest: digest(&self.plan_digest)?, confirmed: self.confirm })
    }
}
#[derive(Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CancelRequest {
    Plan { key: String, plan_digest: String },
    Effect { key: String, plan_digest: String },
    Session { release_for_recovery: Option<bool> },
}
impl CancelRequest {
    pub fn command(self) -> Result<ExcavationCommand> {
        match self {
            Self::Plan { key: value, plan_digest } => {
                key(&value)?;
                Ok(ExcavationCommand::CancelPlan { key: value, digest: digest(&plan_digest)? })
            }
            Self::Effect { key: value, plan_digest } => {
                key(&value)?;
                Ok(ExcavationCommand::CancelEffect { key: value, digest: digest(&plan_digest)? })
            }
            Self::Session { release_for_recovery } =>
                Ok(ExcavationCommand::Release { for_recovery: release_for_recovery.unwrap_or(false) }),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Filter { #[default] All, Unresolved, Terminal }
impl Filter {
    pub fn name(self) -> &'static str {
        match self { Self::All => "all", Self::Unresolved => "unresolved", Self::Terminal => "terminal" }
    }
    pub fn includes(self, entry: &ExcavationEntry) -> bool {
        match self {
            Self::All => true,
            Self::Unresolved => entry.unresolved(),
            Self::Terminal => entry.native().is_some_and(ExcavationRunRecord::terminal),
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum QueryRequest {
    Records { state: Option<Filter>, limit: Option<u32>, continuation: Option<String> },
    Schema {},
}
impl Default for QueryRequest {
    fn default() -> Self { Self::Records { state: None, limit: None, continuation: None } }
}
impl QueryRequest {
    pub fn validate(&self) -> Result<()> {
        if let Self::Records { limit, continuation, .. } = self {
            if !(1..=MAX_PAGE as u32).contains(&limit.unwrap_or(MAX_PAGE as u32)) { return Err(invalid()); }
            if let Some(token) = continuation { digest(token)?; }
        }
        Ok(())
    }
}
