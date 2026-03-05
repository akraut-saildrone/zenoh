//
// Copyright (c) 2026 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//
#![recursion_limit = "256"]

use std::{
    collections::HashMap,
    convert::TryFrom,
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};
use zenoh::{
    internal::{
        bail,
        plugins::{RunningPluginTrait, ZenohPlugin},
        runtime::DynamicRuntime,
        zlock,
    },
    key_expr::{KeyExpr},
    Result as ZResult,
};
use zenoh_plugin_trait::{plugin_long_version, plugin_version, Plugin, PluginControl};
use zenoh_util::ffi::JsonKeyValueMap;

const WORKER_THREAD_NUM: usize = 2;
const MAX_BLOCK_THREAD_NUM: usize = 50;
lazy_static::lazy_static! {
    // The global runtime is used in the dynamic plugins, which we can't get the current runtime
    static ref TOKIO_RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_multi_thread()
               .worker_threads(WORKER_THREAD_NUM)
               .max_blocking_threads(MAX_BLOCK_THREAD_NUM)
               .enable_all()
               .build()
               .expect("Unable to create runtime");
}
#[inline(always)]
fn spawn_runtime(task: impl Future<Output = ()> + Send + 'static) {
    // Check whether able to get the current runtime
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => {
            // Able to get the current runtime (standalone binary), spawn on the current runtime
            rt.spawn(task);
        }
        Err(_) => {
            // Unable to get the current runtime (dynamic plugins), spawn on the global runtime
            TOKIO_RUNTIME.spawn(task);
        }
    }
}

/// Message format for sequence-tracked messages
///
/// This format enables detection of out-of-sequence and missing messages.
/// Publishers MUST include this metadata with each message.
///
/// # JSON Format
/// ```json
/// {
///   "seq": 42,
///   "publisher_id": "sensor-001",
///   "timestamp_ns": 1709571234567890123,
///   "payload": { ... }
/// }
/// ```
///
/// # Binary Format (Custom)
/// For performance-critical applications, use a compact binary format:
/// - Bytes 0-7: sequence number (u64, little-endian)
/// - Bytes 8-15: timestamp in nanoseconds (u64, little-endian)
/// - Bytes 16-47: publisher_id (32 bytes, UTF-8, zero-padded)
/// - Bytes 48+: payload (variable length)
///
/// # Compatibility Notes
/// - `seq` MUST be monotonically increasing per publisher
/// - `publisher_id` MUST be unique and stable across restarts
/// - `timestamp_ns` SHOULD use system monotonic time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencedMessage {
    /// Monotonically increasing sequence number (per publisher)
    pub seq: u64,

    /// Unique identifier for the message publisher
    pub publisher_id: String,

    /// Timestamp in nanoseconds since UNIX epoch (optional but recommended)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_ns: Option<u64>,

    /// The actual message payload (flexible type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Tracks sequence state for a single publisher
#[derive(Debug, Clone)]
struct PublisherState {
    /// Last received sequence number
    last_seq: u64,

    /// Total messages received from this publisher
    message_count: u64,

    /// Count of out-of-order messages detected
    out_of_order_count: u64,

    /// Count of missing messages detected (gaps in sequence)
    missing_count: u64,

    /// Last timestamp received (for temporal analysis)
    last_timestamp_ns: Option<u64>,
}

impl PublisherState {
    fn new() -> Self {
        Self {
            last_seq: 0,
            message_count: 0,
            out_of_order_count: 0,
            missing_count: 0,
            last_timestamp_ns: None,
        }
    }

    /// Process a new sequence number and detect anomalies
    fn process_sequence(&mut self, seq: u64, timestamp_ns: Option<u64>) -> SequenceStatus {
        self.message_count += 1;

        // First message from this publisher
        if self.message_count == 1 {
            self.last_seq = seq;
            self.last_timestamp_ns = timestamp_ns;
            return SequenceStatus::FirstMessage;
        }

        let status = if seq == self.last_seq + 1 {
            // Expected sequence
            SequenceStatus::InSequence
        } else if seq <= self.last_seq {
            // Out of order (duplicate or late arrival)
            self.out_of_order_count += 1;
            if seq == self.last_seq {
                SequenceStatus::Duplicate
            } else {
                SequenceStatus::OutOfOrder {
                    expected: self.last_seq + 1,
                    received: seq,
                }
            }
        } else {
            // Gap detected - missing messages
            let gap = seq - self.last_seq - 1;
            self.missing_count += gap;
            SequenceStatus::Missing {
                expected: self.last_seq + 1,
                received: seq,
                gap_size: gap,
            }
        };

        // Update state (only advance if not duplicate)
        if seq != self.last_seq {
            if seq > self.last_seq {
                self.last_seq = seq;
            }
            self.last_timestamp_ns = timestamp_ns;
        }

        status
    }

    fn get_stats(&self) -> PublisherStats {
        PublisherStats {
            message_count: self.message_count,
            last_seq: self.last_seq,
            out_of_order_count: self.out_of_order_count,
            missing_count: self.missing_count,
        }
    }
}

#[derive(Debug, Clone)]
enum SequenceStatus {
    FirstMessage,
    InSequence,
    Duplicate,
    OutOfOrder { expected: u64, received: u64 },
    Missing { expected: u64, received: u64, gap_size: u64 },
}

#[derive(Debug, Clone, Serialize)]
struct PublisherStats {
    message_count: u64,
    last_seq: u64,
    out_of_order_count: u64,
    missing_count: u64,
}

// The struct implementing the ZenohPlugin trait
pub struct SequenceDetectorPlugin {}

// declaration of the plugin's VTable for zenohd to find the plugin's functions to be called
#[cfg(feature = "dynamic_plugin")]
zenoh_plugin_trait::declare_plugin!(SequenceDetectorPlugin);

// A default selector for sequence detection (in case the config doesn't set it)
const DEFAULT_SELECTOR: &str = "sequenced/**";
const DEFAULT_STATS_INTERVAL_SECS: u64 = 60;

impl ZenohPlugin for SequenceDetectorPlugin {}
impl Plugin for SequenceDetectorPlugin {
    type StartArgs = DynamicRuntime;
    type Instance = zenoh::internal::plugins::RunningPlugin;

    const DEFAULT_NAME: &'static str = "sequence-detector";
    const PLUGIN_VERSION: &'static str = plugin_version!();
    const PLUGIN_LONG_VERSION: &'static str = plugin_long_version!();

    fn start(name: &str, runtime: &Self::StartArgs) -> ZResult<Self::Instance> {
        let config = runtime.get_config().get_plugin_config(name).unwrap();
        let map_cfg = config.as_object().unwrap();

        // Get the selector to monitor
        let selector: KeyExpr = match map_cfg.get("selector") {
            Some(serde_json::Value::String(s)) => KeyExpr::try_from(s)?,
            None => KeyExpr::try_from(DEFAULT_SELECTOR).unwrap(),
            _ => {
                bail!("selector must be a string for {}", name)
            }
        }
        .clone()
        .into_owned();

        // Get stats reporting interval
        let stats_interval_secs: u64 = match map_cfg.get("stats_interval_secs") {
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(DEFAULT_STATS_INTERVAL_SECS),
            None => DEFAULT_STATS_INTERVAL_SECS,
            _ => {
                bail!("stats_interval_secs must be a number for {}", name)
            }
        };

        // Get stats publication key (optional)
        let stats_key: Option<KeyExpr> = match map_cfg.get("stats_key") {
            Some(serde_json::Value::String(s)) => Some(KeyExpr::try_from(s)?.clone().into_owned()),
            None => None,
            _ => {
                bail!("stats_key must be a string for {}", name)
            }
        };

        info!(
            "Starting Sequence Detector Plugin: selector={}, stats_interval={}s",
            selector, stats_interval_secs
        );

        let flag = Arc::new(AtomicBool::new(true));
        spawn_runtime(run(
            runtime.clone(),
            selector,
            stats_interval_secs,
            stats_key,
            flag.clone(),
        ));

        Ok(Box::new(RunningPlugin(Arc::new(Mutex::new(
            RunningPluginInner {
                flag,
                name: name.into(),
                runtime: runtime.clone(),
            },
        )))))
    }
}

struct RunningPluginInner {
    flag: Arc<AtomicBool>,
    name: String,
    #[allow(dead_code)]
    runtime: DynamicRuntime,
}

#[derive(Clone)]
struct RunningPlugin(Arc<Mutex<RunningPluginInner>>);

impl PluginControl for RunningPlugin {}

impl RunningPluginTrait for RunningPlugin {
    fn config_checker(
        &self,
        path: &str,
        old: &JsonKeyValueMap,
        new: &JsonKeyValueMap,
    ) -> ZResult<Option<JsonKeyValueMap>> {
        let old: serde_json::Map<String, serde_json::Value> = old.into();
        let new: serde_json::Map<String, serde_json::Value> = new.into();
        let guard = zlock!(&self.0);

        const SELECTOR: &str = "selector";
        if path == SELECTOR || path.is_empty() {
            match (old.get(SELECTOR), new.get(SELECTOR)) {
                (Some(serde_json::Value::String(os)), Some(serde_json::Value::String(ns)))
                    if os == ns => {}
                (_, Some(serde_json::Value::String(_selector))) => {
                    info!("Sequence detector configuration changed, restart required");
                    return Ok(None);
                }
                (_, None) => {}
                _ => {
                    bail!("selector for {} must be a string", &guard.name)
                }
            }
        }

        Ok(None)
    }
}

impl Drop for RunningPlugin {
    fn drop(&mut self) {
        zlock!(self.0).flag.store(false, Relaxed);
    }
}

async fn run(
    runtime: DynamicRuntime,
    selector: KeyExpr<'static>,
    stats_interval_secs: u64,
    stats_key: Option<KeyExpr<'static>>,
    flag: Arc<AtomicBool>,
) {
    zenoh_util::init_log_from_env_or("info");

    let session = zenoh::session::init(runtime).await.unwrap();

    // Track state per publisher
    let publisher_states: Arc<Mutex<HashMap<String, PublisherState>>> =
        Arc::new(Mutex::new(HashMap::new()));

    debug!("Sequence Detector: subscribing to {}", selector);
    let sub = session.declare_subscriber(&selector).await.unwrap();

    // Stats publisher (optional)
    let stats_publisher = if let Some(key) = stats_key {
        debug!("Sequence Detector: will publish stats to {}", key);
        Some(session.declare_publisher(key).await.unwrap())
    } else {
        None
    };

    // Stats reporting task
    let states_clone = publisher_states.clone();
    let flag_clone = flag.clone();
    spawn_runtime(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(stats_interval_secs));
        while flag_clone.load(Relaxed) {
            interval.tick().await;

            // Collect stats snapshot (drop lock before await)
            let all_stats = {
                let states = states_clone.lock().unwrap();
                if states.is_empty() {
                    None
                } else {
                    info!("=== Sequence Detector Statistics ===");
                    for (publisher_id, state) in states.iter() {
                        let stats = state.get_stats();
                        info!(
                            "Publisher '{}': msgs={}, last_seq={}, out_of_order={}, missing={}",
                            publisher_id,
                            stats.message_count,
                            stats.last_seq,
                            stats.out_of_order_count,
                            stats.missing_count
                        );
                    }

                    Some(
                        states
                            .iter()
                            .map(|(id, state)| (id.clone(), state.get_stats()))
                            .collect::<HashMap<String, PublisherStats>>()
                    )
                }
            }; // Lock dropped here

            // Publish stats if configured (outside lock)
            if let (Some(ref publisher), Some(stats)) = (&stats_publisher, all_stats) {
                if let Ok(json) = serde_json::to_string(&stats) {
                    let _ = publisher.put(json).await;
                }
            }
        }
    });

    // Main message processing loop
    while flag.load(Relaxed) {
        if let Ok(sample) = sub.recv_async().await {
            let key_expr = sample.key_expr().to_string();

            // Try to parse as JSON
            match sample.payload().try_to_string() {
                Ok(payload_str) => {
                    match serde_json::from_str::<SequencedMessage>(&payload_str) {
                        Ok(msg) => {
                            let mut states = publisher_states.lock().unwrap();
                            let state = states
                                .entry(msg.publisher_id.clone())
                                .or_insert_with(PublisherState::new);

                            let status = state.process_sequence(msg.seq, msg.timestamp_ns);

                            match status {
                                SequenceStatus::FirstMessage => {
                                    info!(
                                        "[{}] First message from publisher '{}' with seq={}",
                                        key_expr, msg.publisher_id, msg.seq
                                    );
                                }
                                SequenceStatus::InSequence => {
                                    debug!(
                                        "[{}] In-sequence message from '{}': seq={}",
                                        key_expr, msg.publisher_id, msg.seq
                                    );
                                }
                                SequenceStatus::Duplicate => {
                                    warn!(
                                        "[{}] DUPLICATE from '{}': seq={} (already received)",
                                        key_expr, msg.publisher_id, msg.seq
                                    );
                                }
                                SequenceStatus::OutOfOrder { expected, received } => {
                                    warn!(
                                        "[{}] OUT-OF-ORDER from '{}': expected seq={}, received seq={}",
                                        key_expr, msg.publisher_id, expected, received
                                    );
                                }
                                SequenceStatus::Missing { expected, received, gap_size } => {
                                    error!(
                                        "[{}] MISSING MESSAGES from '{}': expected seq={}, received seq={}, gap={}",
                                        key_expr, msg.publisher_id, expected, received, gap_size
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "[{}] Failed to parse message as SequencedMessage: {}",
                                key_expr, e
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "[{}] Failed to decode payload as string: {}",
                        key_expr, e
                    );
                }
            }
        }
    }

    info!("Sequence Detector plugin stopped");
}
