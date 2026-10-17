use std::{
    io::{Read as _, Write as _},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DiscordConfig {
    pub(crate) enabled: bool,
    pub(crate) application_id: String,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            application_id: String::new(),
        }
    }
}

impl DiscordConfig {
    pub(crate) fn should_connect(&self) -> bool {
        self.enabled && !self.application_id.trim().is_empty()
    }
}

/// Timer phase published through Discord local IPC (US-003/US-004 own wording).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresenceActivity {
    pub(crate) details: String,
    pub(crate) state: String,
    pub(crate) start_timestamp: Option<u64>,
    pub(crate) end_timestamp: Option<u64>,
}

fn presence_activity_payload(
    activity: &PresenceActivity,
    pid: u32,
    nonce: &str,
) -> serde_json::Value {
    let mut timestamps = serde_json::Map::new();
    if let Some(start) = activity.start_timestamp {
        timestamps.insert(
            "start".to_string(),
            serde_json::Value::Number(serde_json::Number::from(start)),
        );
    }
    if let Some(end) = activity.end_timestamp {
        timestamps.insert(
            "end".to_string(),
            serde_json::Value::Number(serde_json::Number::from(end)),
        );
    }
    let mut fields = serde_json::Map::new();
    fields.insert(
        "details".to_string(),
        serde_json::Value::String(activity.details.clone()),
    );
    if !activity.state.is_empty() {
        fields.insert(
            "state".to_string(),
            serde_json::Value::String(activity.state.clone()),
        );
    }
    if !timestamps.is_empty() {
        fields.insert(
            "timestamps".to_string(),
            serde_json::Value::Object(timestamps),
        );
    }
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": fields,
        },
        "nonce": nonce,
    })
}

fn presence_clear_payload(pid: u32, nonce: &str) -> serde_json::Value {
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": null,
        },
        "nonce": nonce,
    })
}

fn encode_discord_ipc_frame(opcode: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn discord_ipc_candidates_from(bases: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for base in bases {
        for index in 0..10 {
            candidates.push(base.join(format!("discord-ipc-{index}")));
        }
    }
    candidates
}

fn discord_ipc_candidates() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let base = PathBuf::from(runtime);
        if !base.as_os_str().is_empty() {
            bases.push(base);
        }
    }
    if let Some(tmpdir) = std::env::var_os("TMPDIR") {
        let base = PathBuf::from(tmpdir);
        if !base.as_os_str().is_empty() {
            bases.push(base);
        }
    }
    bases.push(PathBuf::from("/tmp"));
    let mut seen = std::collections::HashSet::new();
    bases.retain(|base| seen.insert(base.clone()));
    discord_ipc_candidates_from(&bases)
}

#[derive(Debug)]
enum PresenceRequest {
    Update(PresenceActivity),
    Clear,
}

/// Pure countdown projection for US-003.
///
/// Running presence shows `Sleep timer` and the Discord countdown timestamp,
/// without state text so Discord only shows its green live timer below it.
fn should_show_poweroff(power_mode: &str) -> bool {
    power_mode == "poweroff"
}

pub(crate) fn countdown_presence(
    remaining: Duration,
    _power_mode: &str,
    now_unix: u64,
) -> PresenceActivity {
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state: String::new(),
        start_timestamp: None,
        end_timestamp: Some(now_unix.saturating_add(remaining.as_secs())),
    }
}

/// Paused projection: same details line, `paused` state, and no timestamps so
/// Discord never shows a stale live countdown while paused.
pub(crate) fn paused_presence(power_mode: &str) -> PresenceActivity {
    let mut state = "paused".to_string();
    if should_show_poweroff(power_mode) {
        state.push_str(" \u{2022} Power off after timer");
    }
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state,
        start_timestamp: None,
        end_timestamp: None,
    }
}

/// Post-countdown state for runs that do not represent a real sleep interval
/// (`--no-actions`, dry-run, action failures, or no enabled actions). Kept as
/// `finished` with no timestamps so Discord never shows a misleading elapsed
/// sleep timer.
pub(crate) fn post_countdown_presence() -> PresenceActivity {
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state: "finished".to_string(),
        start_timestamp: None,
        end_timestamp: None,
    }
}

/// Post-countdown elapsed timestamp for successful real-action runs (US-004).
///
/// `sleep_start_unix` is captured at the start of post-countdown actions. The
/// state label replaces the previous `finished` state, and the green elapsed
/// counter comes from `start_timestamp`.
pub(crate) fn sleeping_presence(sleep_start_unix: u64) -> PresenceActivity {
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state: "Sleeping for:".to_string(),
        start_timestamp: Some(sleep_start_unix),
        end_timestamp: None,
    }
}

/// Pure post-run presence selector (US-004).
///
/// Only a real, fully successful action run represents a sleep interval and
/// gets the green elapsed timestamp with the `Sleeping for:` state. Dry-run, `--no-actions`,
/// failures, and empty action lists keep the accurate timestamp-free `finished`
/// state so Discord never shows a misleading sleep timer.
pub(crate) fn post_run_presence(
    dry_run: bool,
    no_actions: bool,
    has_failures: bool,
    has_actions: bool,
    sleep_start_unix: u64,
) -> PresenceActivity {
    if !dry_run && !no_actions && !has_failures && has_actions {
        sleeping_presence(sleep_start_unix)
    } else {
        post_countdown_presence()
    }
}

/// Non-blocking Discord Rich Presence client over local IPC.
///
/// `start` never blocks the caller: the socket handshake runs on a background
/// thread with short timeouts, and `update`/`clear` only enqueue on an
/// unbounded channel. Missing Discord, a missing socket, or any I/O failure
/// is swallowed so timer runs never fail or stall. Dropping the client closes
/// the channel; the worker attempts one final clear in the background.
#[derive(Debug)]
pub(crate) struct DiscordPresence {
    sender: Option<mpsc::Sender<PresenceRequest>>,
    clear_on_drop: bool,
}

impl DiscordPresence {
    pub(crate) fn start(config: &DiscordConfig) -> Self {
        if !config.should_connect() {
            return Self {
                sender: None,
                clear_on_drop: true,
            };
        }
        let application_id = config.application_id.trim().to_string();
        if application_id.is_empty() {
            return Self {
                sender: None,
                clear_on_drop: true,
            };
        }
        let (sender, requests) = mpsc::channel();
        thread::spawn(move || discord_presence_worker(&application_id, requests));
        Self {
            sender: Some(sender),
            clear_on_drop: true,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn inactive() -> Self {
        Self {
            sender: None,
            clear_on_drop: true,
        }
    }

    // Used by tests to assert no-op vs active; US-003/004 will branch on it.
    #[allow(dead_code)]
    pub(crate) fn is_active(&self) -> bool {
        self.sender.is_some()
    }

    // Countdown (US-003) and post-countdown (US-004) updates; non-blocking.
    pub(crate) fn update(&self, activity: PresenceActivity) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(PresenceRequest::Update(activity));
        }
    }

    pub(crate) fn clear(&self) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(PresenceRequest::Clear);
        }
    }

    /// Stop the automatic clear on drop. Used on successful completion so the
    /// countdown transitions to the post-countdown state instead of clearing.
    pub(crate) fn disarm(&mut self) {
        self.clear_on_drop = false;
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        // Best-effort clear in the background, then close the channel so the
        // worker exits. No join here so countdown/action paths never block.
        // Disarmed on success so the post-countdown state is retained.
        if self.clear_on_drop {
            self.clear();
        }
        self.sender.take();
    }
}

fn discord_presence_worker(application_id: &str, requests: mpsc::Receiver<PresenceRequest>) {
    let mut stream = discord_connect(application_id);
    let pid = std::process::id();
    for request in requests {
        let Some(socket) = stream.as_mut() else {
            continue;
        };
        let payload = match &request {
            PresenceRequest::Update(activity) => {
                presence_activity_payload(activity, pid, &next_presence_nonce())
            }
            PresenceRequest::Clear => presence_clear_payload(pid, &next_presence_nonce()),
        };
        if discord_send_activity(socket, &payload).is_err() {
            stream = None;
        }
    }
    // No final clear here: `Drop` already sends an explicit `Clear` when the
    // run is stopped, and a disarmed guard (successful completion) must retain
    // the post-countdown state instead of wiping it. Discord clears stale
    // presence itself once the process exits and the socket closes.
}

fn discord_connect(application_id: &str) -> Option<std::os::unix::net::UnixStream> {
    use std::os::unix::net::UnixStream;
    for path in discord_ipc_candidates() {
        let Ok(stream) = UnixStream::connect(&path) else {
            continue;
        };
        if discord_handshake(&stream, application_id).is_ok() {
            return Some(stream);
        }
    }
    None
}

fn discord_handshake(
    stream: &std::os::unix::net::UnixStream,
    application_id: &str,
) -> std::io::Result<()> {
    let mut stream = stream.try_clone()?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let payload = serde_json::json!({
        "v": 1,
        "client_id": application_id,
    });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let frame = encode_discord_ipc_frame(0, &bytes);
    stream.write_all(&frame)?;
    // Best effort: consume the READY reply when Discord answers, but a missing
    // or slow reply must not fail the worker; updates will fail later if the
    // handshake did not actually succeed.
    let mut header = [0_u8; 8];
    match stream.read_exact(&mut header) {
        Ok(()) => {
            let len = u32::from_le_bytes(header[4..8].try_into().unwrap_or([0; 4])) as usize;
            if len <= 64 * 1024 {
                let mut body = vec![0_u8; len];
                let _ = stream.read_exact(&mut body);
            }
            Ok(())
        }
        Err(err)
            if err.kind() == std::io::ErrorKind::WouldBlock
                || err.kind() == std::io::ErrorKind::TimedOut =>
        {
            Ok(())
        }
        Err(_) => Ok(()),
    }
}

fn discord_send_activity(
    stream: &mut std::os::unix::net::UnixStream,
    payload: &serde_json::Value,
) -> std::io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let bytes = serde_json::to_vec(payload)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let frame = encode_discord_ipc_frame(1, &bytes);
    stream.write_all(&frame)?;
    Ok(())
}

fn next_presence_nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("gosleep-{}-{count}", unix_timestamp())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn discord_activity_payload_formats_details_state_and_timestamps() {
        let activity = PresenceActivity {
            details: "Sleep timer".to_string(),
            state: String::new(),
            start_timestamp: Some(1_700_000_000),
            end_timestamp: Some(1_700_002_700),
        };
        let payload = presence_activity_payload(&activity, 1234, "nonce-1");
        assert_eq!(payload["cmd"], "SET_ACTIVITY");
        assert_eq!(payload["args"]["pid"], 1234);
        assert_eq!(payload["args"]["activity"]["details"], "Sleep timer");
        assert!(payload["args"]["activity"].get("state").is_none());
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["start"],
            1_700_000_000
        );
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["end"],
            1_700_002_700
        );
        assert_eq!(payload["nonce"], "nonce-1");

        let minimal = PresenceActivity {
            details: "Sleep timer".to_string(),
            state: "paused".to_string(),
            start_timestamp: None,
            end_timestamp: None,
        };
        let payload = presence_activity_payload(&minimal, 1234, "nonce-2");
        assert_eq!(payload["args"]["activity"]["details"], "Sleep timer");
        assert_eq!(payload["args"]["activity"]["state"], "paused");
        assert!(
            payload["args"]["activity"].get("timestamps").is_none(),
            "paused state must not leave a stale countdown timestamp"
        );

        let clear = presence_clear_payload(1234, "nonce-3");
        assert_eq!(clear["cmd"], "SET_ACTIVITY");
        assert!(clear["args"]["activity"].is_null());
        assert_eq!(clear["nonce"], "nonce-3");
    }

    #[test]
    fn discord_ipc_frame_encodes_opcode_and_length_little_endian() {
        let frame = encode_discord_ipc_frame(1, b"{}");
        assert_eq!(frame, vec![1, 0, 0, 0, 2, 0, 0, 0, b'{', b'}']);

        let handshake = encode_discord_ipc_frame(0, b"hi");
        assert_eq!(&handshake[0..4], &0_u32.to_le_bytes());
        assert_eq!(&handshake[4..8], &2_u32.to_le_bytes());
        assert_eq!(&handshake[8..], b"hi");
    }

    #[test]
    fn discord_ipc_candidates_cover_runtime_and_tmp_sockets() {
        let bases = vec![PathBuf::from("/run/user/1000"), PathBuf::from("/tmp")];
        let candidates = discord_ipc_candidates_from(&bases);
        assert_eq!(candidates.len(), 20);
        assert_eq!(candidates[0], PathBuf::from("/run/user/1000/discord-ipc-0"));
        assert_eq!(candidates[9], PathBuf::from("/run/user/1000/discord-ipc-9"));
        assert_eq!(candidates[10], PathBuf::from("/tmp/discord-ipc-0"));
        assert_eq!(candidates[19], PathBuf::from("/tmp/discord-ipc-9"));
    }

    #[test]
    fn discord_disabled_or_unconfigured_is_noop_without_blocking() {
        for config in [
            DiscordConfig {
                enabled: false,
                application_id: "123".to_string(),
            },
            DiscordConfig {
                enabled: true,
                application_id: String::new(),
            },
            DiscordConfig {
                enabled: true,
                application_id: "   ".to_string(),
            },
        ] {
            assert!(!config.should_connect());
            let started = Instant::now();
            let presence = DiscordPresence::start(&config);
            assert!(!presence.is_active());
            presence.update(PresenceActivity {
                details: "Sleep timer".to_string(),
                state: String::new(),
                start_timestamp: None,
                end_timestamp: None,
            });
            presence.clear();
            drop(presence);
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "disabled/unconfigured presence must be an instant no-op"
            );
        }
    }

    #[test]
    fn countdown_presence_shows_only_details_and_end_timestamp() {
        let remaining = Duration::from_secs(45 * 60);
        let activity = countdown_presence(remaining, "none", 1_700_000_000);
        assert_eq!(activity.details, "Sleep timer");
        assert!(activity.state.is_empty());
        assert_eq!(activity.start_timestamp, None);
        assert_eq!(
            activity.end_timestamp,
            Some(1_700_000_000 + remaining.as_secs())
        );

        // Payload keeps the live countdown end timestamp.
        let payload = presence_activity_payload(&activity, 1234, "nonce-c1");
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["end"],
            1_700_000_000 + remaining.as_secs()
        );
        assert!(payload["args"]["activity"].get("state").is_none());
    }

    #[test]
    fn paused_presence_has_no_stale_countdown_timestamp() {
        let activity = paused_presence("none");
        assert_eq!(activity.details, "Sleep timer");
        assert_eq!(activity.state, "paused");
        assert_eq!(activity.start_timestamp, None);
        assert_eq!(activity.end_timestamp, None);

        let payload = presence_activity_payload(&activity, 1234, "nonce-p1");
        assert!(
            payload["args"]["activity"].get("timestamps").is_none(),
            "paused state must not leave a stale countdown timestamp"
        );
    }

    #[test]
    fn countdown_never_adds_state_text_for_poweroff() {
        let running_off = countdown_presence(Duration::from_secs(60), "poweroff", 1000);
        assert!(running_off.state.is_empty());
        let paused_off = paused_presence("poweroff");
        assert!(paused_off.state.contains("Power off after timer"));

        for mode in [
            "none",
            "suspend",
            "hibernate",
            "hybrid-sleep",
            "reboot",
            "",
            "PowerOff",
            "POWEROFF",
        ] {
            let running = countdown_presence(Duration::from_secs(60), mode, 1000);
            assert!(
                running.state.is_empty(),
                "mode {mode:?} must not add countdown state text, got {:?}",
                running.state
            );
            let paused = paused_presence(mode);
            assert!(
                !paused.state.contains("Power off after timer"),
                "mode {mode:?} must omit the indicator when paused, got {:?}",
                paused.state
            );
        }
    }

    #[test]
    fn sleeping_presence_labels_elapsed_start_without_end() {
        let activity = sleeping_presence(1_700_000_000);
        assert_eq!(activity.details, "Sleep timer");
        assert_eq!(activity.state, "Sleeping for:");
        assert_eq!(activity.start_timestamp, Some(1_700_000_000));
        assert_eq!(activity.end_timestamp, None);

        let payload = presence_activity_payload(&activity, 1234, "nonce-s1");
        assert_eq!(payload["args"]["activity"]["state"], "Sleeping for:");
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["start"],
            1_700_000_000
        );
        assert!(
            payload["args"]["activity"]["timestamps"]
                .get("end")
                .is_none(),
            "sleeping state must not carry a countdown end timestamp"
        );
    }

    #[test]
    fn post_run_presence_only_claims_sleep_for_real_success() {
        let sleeping = post_run_presence(false, false, false, true, 1000);
        assert_eq!(sleeping.state, "Sleeping for:");
        assert_eq!(sleeping.start_timestamp, Some(1000));
        assert_eq!(sleeping.end_timestamp, None);

        // Every non-sleep case keeps the accurate timestamp-free finished state.
        for (dry_run, no_actions) in [(true, false), (false, true), (true, true)] {
            let activity = post_run_presence(dry_run, no_actions, false, true, 1000);
            assert_eq!(activity, post_countdown_presence());
        }
        assert_eq!(
            post_run_presence(false, false, true, true, 1000),
            post_countdown_presence(),
            "failures must not claim a sleep interval"
        );
        assert_eq!(
            post_run_presence(false, false, false, false, 1000),
            post_countdown_presence(),
            "empty action list must not claim a sleep interval"
        );
    }
}
