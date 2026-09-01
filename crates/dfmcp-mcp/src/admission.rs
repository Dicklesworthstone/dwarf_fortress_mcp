#![forbid(unsafe_code)]

//! Single-use process admission for the authenticated live MCP server.
//!
//! The external launcher proves the exact compatibility tuple and release
//! binary before `exec`. This module makes that launcher boundary mandatory for
//! the Rust process: direct `serve-live` invocation fails closed unless a
//! bounded, owner-private, process- and inode-bound ticket is consumed first.
//! The ticket is an accidental-bypass and stale-launch fence within the stated
//! same-host threat model; it is not a defense against a compromised account
//! that can replace the process, launcher, and ticket together.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{File, Metadata, remove_file, symlink_metadata};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use dfmcp_core::Digest32;
use serde::Deserialize;
use serde_json::{Number, Value};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const TICKET_ENVIRONMENT_VARIABLE: &str = "DFMCP_ADMISSION_TICKET";
const TICKET_SCHEMA: &str = "dfmcp.live-admission-ticket/1";
const AUTHORIZED_STATE: &str = "authorized_to_exec";
const READ_ONLY_MODE: &str = "authenticated_live_read_only";
const MAX_TICKET_BYTES: u64 = 64 * 1024;
const MAX_TICKET_LIFETIME_SECONDS: u64 = 300;
const CLOCK_SKEW_SECONDS: u64 = 5;
const REQUIRED_CAPABILITIES: [&str; 4] = ["doctor", "observe", "query", "wait"];

static ADMISSION_PROVENANCE: OnceLock<AdmissionProvenance> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionProvenance {
    ticket_id: String,
    compatibility_entry_id: String,
    compatibility_registry_digest: String,
    compatibility_decision_digest: String,
    server_receipt_digest: String,
    launch_digest: String,
    server_binary_sha256: String,
}

impl AdmissionProvenance {
    #[must_use]
    pub fn ticket_id(&self) -> &str {
        &self.ticket_id
    }

    #[must_use]
    pub fn compatibility_entry_id(&self) -> &str {
        &self.compatibility_entry_id
    }

    #[must_use]
    pub fn compatibility_registry_digest(&self) -> &str {
        &self.compatibility_registry_digest
    }

    #[must_use]
    pub fn compatibility_decision_digest(&self) -> &str {
        &self.compatibility_decision_digest
    }

    #[must_use]
    pub fn server_receipt_digest(&self) -> &str {
        &self.server_receipt_digest
    }

    #[must_use]
    pub fn launch_digest(&self) -> &str {
        &self.launch_digest
    }

    #[must_use]
    pub fn server_binary_sha256(&self) -> &str {
        &self.server_binary_sha256
    }
}

#[must_use]
pub fn current_admission_provenance() -> Option<&'static AdmissionProvenance> {
    ADMISSION_PROVENANCE.get()
}

#[derive(Debug)]
pub struct AdmissionError {
    message: String,
}

impl AdmissionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AdmissionError {}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AdmissionTicket {
    schema: String,
    state: String,
    ticket_id: String,
    process_id: u32,
    created_unix_seconds: u64,
    expires_unix_seconds: u64,
    compatibility_entry_id: String,
    compatibility_registry_digest: String,
    compatibility_decision_digest: String,
    server_receipt_digest: String,
    launch_digest: String,
    server_binary_sha256: String,
    server_binary_device: u64,
    server_binary_inode: u64,
    server_binary_bytes: u64,
    server_binary_mode: u32,
    server_binary_owner_uid: u32,
    mode: String,
    capabilities: Vec<String>,
    mutation_capabilities: Vec<String>,
    ticket_digest: String,
}

struct AdmissionContext<'a> {
    now_unix_seconds: u64,
    process_id: u32,
    executable_path: &'a Path,
}

fn invalid(message: impl Into<String>) -> AdmissionError {
    AdmissionError::new(message)
}

fn validate_hash(value: &str, field: &str) -> Result<(), AdmissionError> {
    let parsed = Digest32::from_hex(value)
        .ok_or_else(|| invalid(format!("admission ticket {field} is not a SHA-256 digest")))?;
    if parsed.to_hex() != value {
        return Err(invalid(format!(
            "admission ticket {field} is not canonical lowercase hexadecimal"
        )));
    }
    Ok(())
}

fn number(value: u64) -> Value {
    Value::Number(Number::from(value))
}

fn unsigned_ticket_value(ticket: &AdmissionTicket) -> Value {
    let mut fields = BTreeMap::new();
    fields.insert("capabilities".to_owned(), Value::Array(
        ticket.capabilities.iter().cloned().map(Value::String).collect(),
    ));
    fields.insert(
        "compatibility_decision_digest".to_owned(),
        Value::String(ticket.compatibility_decision_digest.clone()),
    );
    fields.insert(
        "compatibility_entry_id".to_owned(),
        Value::String(ticket.compatibility_entry_id.clone()),
    );
    fields.insert(
        "compatibility_registry_digest".to_owned(),
        Value::String(ticket.compatibility_registry_digest.clone()),
    );
    fields.insert(
        "created_unix_seconds".to_owned(),
        number(ticket.created_unix_seconds),
    );
    fields.insert(
        "expires_unix_seconds".to_owned(),
        number(ticket.expires_unix_seconds),
    );
    fields.insert(
        "launch_digest".to_owned(),
        Value::String(ticket.launch_digest.clone()),
    );
    fields.insert("mode".to_owned(), Value::String(ticket.mode.clone()));
    fields.insert(
        "mutation_capabilities".to_owned(),
        Value::Array(
            ticket
                .mutation_capabilities
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    fields.insert("process_id".to_owned(), number(u64::from(ticket.process_id)));
    fields.insert("schema".to_owned(), Value::String(ticket.schema.clone()));
    fields.insert(
        "server_binary_bytes".to_owned(),
        number(ticket.server_binary_bytes),
    );
    fields.insert(
        "server_binary_device".to_owned(),
        number(ticket.server_binary_device),
    );
    fields.insert(
        "server_binary_inode".to_owned(),
        number(ticket.server_binary_inode),
    );
    fields.insert(
        "server_binary_mode".to_owned(),
        number(u64::from(ticket.server_binary_mode)),
    );
    fields.insert(
        "server_binary_owner_uid".to_owned(),
        number(u64::from(ticket.server_binary_owner_uid)),
    );
    fields.insert(
        "server_binary_sha256".to_owned(),
        Value::String(ticket.server_binary_sha256.clone()),
    );
    fields.insert(
        "server_receipt_digest".to_owned(),
        Value::String(ticket.server_receipt_digest.clone()),
    );
    fields.insert("state".to_owned(), Value::String(ticket.state.clone()));
    fields.insert(
        "ticket_id".to_owned(),
        Value::String(ticket.ticket_id.clone()),
    );
    Value::Object(fields.into_iter().collect())
}

fn expected_ticket_digest(ticket: &AdmissionTicket) -> Result<String, AdmissionError> {
    let canonical = serde_json::to_vec(&unsigned_ticket_value(ticket)).map_err(|source| {
        invalid(format!(
            "cannot canonicalize admission ticket for digest verification: {source}"
        ))
    })?;
    Ok(Digest32::of_bytes(&canonical).to_hex())
}

fn validate_ticket_semantics(
    ticket: &AdmissionTicket,
    context: &AdmissionContext<'_>,
) -> Result<(), AdmissionError> {
    if ticket.schema != TICKET_SCHEMA {
        return Err(invalid("admission ticket schema is unsupported"));
    }
    if ticket.state != AUTHORIZED_STATE {
        return Err(invalid("admission ticket is not authorized to execute"));
    }
    if ticket.mode != READ_ONLY_MODE {
        return Err(invalid("admission ticket does not select read-only live mode"));
    }
    for (value, field) in [
        (&ticket.ticket_id, "ticket_id"),
        (&ticket.compatibility_entry_id, "compatibility_entry_id"),
        (
            &ticket.compatibility_registry_digest,
            "compatibility_registry_digest",
        ),
        (
            &ticket.compatibility_decision_digest,
            "compatibility_decision_digest",
        ),
        (&ticket.server_receipt_digest, "server_receipt_digest"),
        (&ticket.launch_digest, "launch_digest"),
        (&ticket.server_binary_sha256, "server_binary_sha256"),
        (&ticket.ticket_digest, "ticket_digest"),
    ] {
        validate_hash(value, field)?;
    }
    if ticket.ticket_digest != expected_ticket_digest(ticket)? {
        return Err(invalid(
            "admission ticket digest does not reproduce its canonical fields",
        ));
    }
    if ticket.process_id != context.process_id {
        return Err(invalid(
            "admission ticket is bound to a different operating-system process",
        ));
    }
    if ticket.expires_unix_seconds <= ticket.created_unix_seconds
        || ticket
            .expires_unix_seconds
            .saturating_sub(ticket.created_unix_seconds)
            > MAX_TICKET_LIFETIME_SECONDS
    {
        return Err(invalid("admission ticket lifetime is invalid"));
    }
    if ticket.created_unix_seconds > context.now_unix_seconds.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err(invalid("admission ticket was created in the future"));
    }
    if context.now_unix_seconds > ticket.expires_unix_seconds {
        return Err(invalid("admission ticket has expired"));
    }
    let expected_capabilities = REQUIRED_CAPABILITIES
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    if ticket.capabilities != expected_capabilities {
        return Err(invalid(
            "admission ticket capability set or canonical order drifted",
        ));
    }
    if !ticket.mutation_capabilities.is_empty() {
        return Err(invalid("admission ticket contains mutation capability"));
    }
    if ticket.server_binary_bytes == 0 {
        return Err(invalid("admission ticket names an empty server binary"));
    }
    validate_executable_identity(ticket, context.executable_path)
}

#[cfg(unix)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
}

#[cfg(not(unix))]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.permissions().readonly() == right.permissions().readonly()
}

#[cfg(unix)]
fn validate_private_ticket_path(path: &Path, metadata: &Metadata) -> Result<(), AdmissionError> {
    if metadata.mode() & 0o777 != 0o600 {
        return Err(invalid(
            "admission ticket must have exact owner-read/write mode 0600",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("admission ticket has no parent directory"))?;
    let parent_metadata = symlink_metadata(parent).map_err(|source| {
        invalid(format!(
            "cannot inspect admission ticket directory {}: {source}",
            parent.display()
        ))
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(invalid(
            "admission ticket parent must be a real directory, not a symbolic link",
        ));
    }
    if parent_metadata.mode() & 0o077 != 0 {
        return Err(invalid(
            "admission ticket directory must deny all group and world permissions",
        ));
    }
    if parent_metadata.uid() != metadata.uid() {
        return Err(invalid(
            "admission ticket and its private directory have different owners",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_ticket_path(_path: &Path, _metadata: &Metadata) -> Result<(), AdmissionError> {
    Err(invalid(
        "admitted live startup currently requires Unix metadata and descriptor semantics",
    ))
}

#[cfg(unix)]
fn validate_executable_identity(
    ticket: &AdmissionTicket,
    executable_path: &Path,
) -> Result<(), AdmissionError> {
    let metadata = executable_path.metadata().map_err(|source| {
        invalid(format!(
            "cannot inspect current executable {}: {source}",
            executable_path.display()
        ))
    })?;
    if metadata.dev() != ticket.server_binary_device
        || metadata.ino() != ticket.server_binary_inode
        || metadata.len() != ticket.server_binary_bytes
        || metadata.mode() != ticket.server_binary_mode
        || metadata.uid() != ticket.server_binary_owner_uid
    {
        return Err(invalid(
            "current executable inode does not match the admitted server binary",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_executable_identity(
    _ticket: &AdmissionTicket,
    _executable_path: &Path,
) -> Result<(), AdmissionError> {
    Err(invalid(
        "admitted live startup currently requires Unix executable identity metadata",
    ))
}

fn read_stable_ticket(path: &Path) -> Result<Vec<u8>, AdmissionError> {
    if !path.is_absolute() {
        return Err(invalid("admission ticket path must be absolute"));
    }
    let before = symlink_metadata(path).map_err(|source| {
        invalid(format!(
            "cannot inspect admission ticket {}: {source}",
            path.display()
        ))
    })?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(invalid(
            "admission ticket must be a regular non-symbolic-link file",
        ));
    }
    if before.len() == 0 || before.len() > MAX_TICKET_BYTES {
        return Err(invalid(format!(
            "admission ticket must contain 1..={MAX_TICKET_BYTES} bytes"
        )));
    }
    validate_private_ticket_path(path, &before)?;

    let mut file = File::open(path).map_err(|source| {
        invalid(format!(
            "cannot open admission ticket {}: {source}",
            path.display()
        ))
    })?;
    let opened = file.metadata().map_err(|source| {
        invalid(format!(
            "cannot inspect opened admission ticket {}: {source}",
            path.display()
        ))
    })?;
    if !same_file_identity(&before, &opened) {
        return Err(invalid(
            "admission ticket changed between path inspection and open",
        ));
    }

    let mut bytes = Vec::with_capacity(
        usize::try_from(opened.len())
            .map_err(|_| invalid("admission ticket length does not fit memory bounds"))?,
    );
    (&mut file)
        .take(MAX_TICKET_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| invalid(format!("cannot read admission ticket: {source}")))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_TICKET_BYTES) {
        return Err(invalid("admission ticket exceeded its byte bound while read"));
    }
    let after = file.metadata().map_err(|source| {
        invalid(format!(
            "cannot reinspect opened admission ticket {}: {source}",
            path.display()
        ))
    })?;
    if !same_file_identity(&opened, &after)
        || u64::try_from(bytes.len()).map_or(true, |length| length != after.len())
    {
        return Err(invalid("admission ticket changed while being read"));
    }
    Ok(bytes)
}

fn consume_admission_ticket_at(
    path: &Path,
    context: &AdmissionContext<'_>,
) -> Result<AdmissionProvenance, AdmissionError> {
    let bytes = read_stable_ticket(path)?;
    let ticket: AdmissionTicket = serde_json::from_slice(&bytes)
        .map_err(|source| invalid(format!("cannot parse admission ticket: {source}")))?;
    validate_ticket_semantics(&ticket, context)?;
    remove_file(path).map_err(|source| {
        invalid(format!(
            "cannot consume admission ticket {}: {source}",
            path.display()
        ))
    })?;
    match symlink_metadata(path) {
        Err(source) if source.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(invalid(format!(
                "cannot verify admission ticket consumption: {source}"
            )));
        }
        Ok(_) => {
            return Err(invalid(
                "admission ticket still exists after single-use consumption",
            ));
        }
    }
    Ok(AdmissionProvenance {
        ticket_id: ticket.ticket_id,
        compatibility_entry_id: ticket.compatibility_entry_id,
        compatibility_registry_digest: ticket.compatibility_registry_digest,
        compatibility_decision_digest: ticket.compatibility_decision_digest,
        server_receipt_digest: ticket.server_receipt_digest,
        launch_digest: ticket.launch_digest,
        server_binary_sha256: ticket.server_binary_sha256,
    })
}

fn consume_admission_ticket_from_environment() -> Result<AdmissionProvenance, AdmissionError> {
    let raw = env::var_os(TICKET_ENVIRONMENT_VARIABLE).ok_or_else(|| {
        invalid(format!(
            "{TICKET_ENVIRONMENT_VARIABLE} is required; start live mode through scripts/serve_admitted_live.py"
        ))
    })?;
    let path = PathBuf::from(raw);
    let now_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("system clock precedes the Unix epoch"))?
        .as_secs();
    let executable_path = env::current_exe()
        .map_err(|source| invalid(format!("cannot resolve current executable: {source}")))?;
    consume_admission_ticket_at(
        &path,
        &AdmissionContext {
            now_unix_seconds,
            process_id: std::process::id(),
            executable_path: &executable_path,
        },
    )
}

/// Consume the launcher's single-use ticket, then run the private live server.
pub fn run_live_stdio() {
    let provenance = match consume_admission_ticket_from_environment() {
        Ok(value) => value,
        Err(failure) => {
            eprintln!("admitted live startup: FAIL: {failure}");
            std::process::exit(1);
        }
    };
    if ADMISSION_PROVENANCE.set(provenance).is_err() {
        eprintln!("admitted live startup: FAIL: admission provenance was already initialized");
        std::process::exit(1);
    }
    crate::live_server::run_live_stdio();
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs::{OpenOptions, create_dir, set_permissions};
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
        ticket: PathBuf,
        now: u64,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ignored = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture() -> Result<(Fixture, AdmissionTicket), Box<dyn Error>> {
        let ordinal = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "dfmcp-admission-test-{}-{ordinal}",
            std::process::id()
        ));
        create_dir(&root)?;
        set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let executable = root.join("dwarf-fortress-mcp");
        let mut executable_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&executable)?;
        executable_file.write_all(b"fixture admitted executable")?;
        executable_file.sync_all()?;
        set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
        let metadata = executable.metadata()?;
        let now = 1_800_000_000;
        let ticket_path = root.join("ticket.json");
        let mut ticket = AdmissionTicket {
            schema: TICKET_SCHEMA.to_owned(),
            state: AUTHORIZED_STATE.to_owned(),
            ticket_id: Digest32::of_bytes(b"ticket-id").to_hex(),
            process_id: std::process::id(),
            created_unix_seconds: now.saturating_sub(1),
            expires_unix_seconds: now.saturating_add(120),
            compatibility_entry_id: Digest32::of_bytes(b"entry").to_hex(),
            compatibility_registry_digest: Digest32::of_bytes(b"registry").to_hex(),
            compatibility_decision_digest: Digest32::of_bytes(b"decision").to_hex(),
            server_receipt_digest: Digest32::of_bytes(b"server-receipt").to_hex(),
            launch_digest: Digest32::of_bytes(b"launch").to_hex(),
            server_binary_sha256: Digest32::of_bytes(b"binary").to_hex(),
            server_binary_device: metadata.dev(),
            server_binary_inode: metadata.ino(),
            server_binary_bytes: metadata.len(),
            server_binary_mode: metadata.mode(),
            server_binary_owner_uid: metadata.uid(),
            mode: READ_ONLY_MODE.to_owned(),
            capabilities: REQUIRED_CAPABILITIES
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            mutation_capabilities: Vec::new(),
            ticket_digest: String::new(),
        };
        ticket.ticket_digest = expected_ticket_digest(&ticket)?;
        Ok((
            Fixture {
                root,
                executable,
                ticket: ticket_path,
                now,
            },
            ticket,
        ))
    }

    fn write_ticket(path: &Path, ticket: &AdmissionTicket) -> Result<(), Box<dyn Error>> {
        let mut fields = match unsigned_ticket_value(ticket) {
            Value::Object(value) => value,
            _ => return Err("canonical admission ticket is not an object".into()),
        };
        fields.insert(
            "ticket_digest".to_owned(),
            Value::String(ticket.ticket_digest.clone()),
        );
        let bytes = serde_json::to_vec(&Value::Object(fields))?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    }

    fn context<'a>(fixture: &'a Fixture) -> AdmissionContext<'a> {
        AdmissionContext {
            now_unix_seconds: fixture.now,
            process_id: std::process::id(),
            executable_path: &fixture.executable,
        }
    }

    #[test]
    fn valid_ticket_is_consumed_exactly_once() -> Result<(), Box<dyn Error>> {
        let (fixture, ticket) = fixture()?;
        write_ticket(&fixture.ticket, &ticket)?;
        let provenance = consume_admission_ticket_at(&fixture.ticket, &context(&fixture))?;
        assert_eq!(provenance.ticket_id(), ticket.ticket_id);
        assert_eq!(
            provenance.compatibility_entry_id(),
            ticket.compatibility_entry_id
        );
        assert!(!fixture.ticket.exists());
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        Ok(())
    }

    #[test]
    fn expired_and_cross_process_tickets_fail_closed() -> Result<(), Box<dyn Error>> {
        let (fixture, mut ticket) = fixture()?;
        ticket.expires_unix_seconds = fixture.now.saturating_sub(1);
        ticket.ticket_digest = expected_ticket_digest(&ticket)?;
        write_ticket(&fixture.ticket, &ticket)?;
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        remove_file(&fixture.ticket)?;

        ticket.expires_unix_seconds = fixture.now.saturating_add(120);
        ticket.process_id = ticket.process_id.saturating_add(1);
        ticket.ticket_digest = expected_ticket_digest(&ticket)?;
        write_ticket(&fixture.ticket, &ticket)?;
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        Ok(())
    }

    #[test]
    fn mutation_capability_and_inode_drift_are_rejected() -> Result<(), Box<dyn Error>> {
        let (fixture, mut ticket) = fixture()?;
        ticket.mutation_capabilities.push("pause".to_owned());
        ticket.ticket_digest = expected_ticket_digest(&ticket)?;
        write_ticket(&fixture.ticket, &ticket)?;
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        remove_file(&fixture.ticket)?;

        ticket.mutation_capabilities.clear();
        ticket.server_binary_inode = ticket.server_binary_inode.saturating_add(1);
        ticket.ticket_digest = expected_ticket_digest(&ticket)?;
        write_ticket(&fixture.ticket, &ticket)?;
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        Ok(())
    }

    #[test]
    fn permissive_or_symbolic_ticket_paths_are_rejected() -> Result<(), Box<dyn Error>> {
        let (fixture, ticket) = fixture()?;
        write_ticket(&fixture.ticket, &ticket)?;
        set_permissions(&fixture.ticket, std::fs::Permissions::from_mode(0o640))?;
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        remove_file(&fixture.ticket)?;

        let real = fixture.root.join("real-ticket.json");
        write_ticket(&real, &ticket)?;
        symlink(&real, &fixture.ticket)?;
        assert!(consume_admission_ticket_at(&fixture.ticket, &context(&fixture)).is_err());
        Ok(())
    }
}
