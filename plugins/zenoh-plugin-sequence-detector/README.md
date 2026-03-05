# Zenoh Sequence Detector Plugin

A Zenoh plugin that monitors message streams for out-of-sequence and missing messages. This plugin helps detect network reliability issues, publisher bugs, and message loss in distributed systems.

## Features

- **Sequence Tracking**: Monitors monotonically increasing sequence numbers per publisher
- **Missing Message Detection**: Identifies gaps in sequence numbers
- **Out-of-Order Detection**: Detects messages arriving after later sequences
- **Duplicate Detection**: Flags repeated sequence numbers
- **Per-Publisher Statistics**: Tracks metrics separately for each message source
- **Periodic Reporting**: Configurable stats logging and publication
- **Real-time Alerts**: Immediate warnings for anomalies (log output)

## Message Format

Publishers must use the following JSON format for messages to be tracked:

### JSON Schema

```json
{
  "seq": 42,
  "publisher_id": "sensor-001",
  "timestamp_ns": 1709571234567890123,
  "payload": {
    "temperature": 22.5,
    "humidity": 65
  }
}
```

### Field Descriptions

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `seq` | u64 | **Yes** | Monotonically increasing sequence number (per publisher) |
| `publisher_id` | string | **Yes** | Unique identifier for the message publisher |
| `timestamp_ns` | u64 | No | Timestamp in nanoseconds since UNIX epoch |
| `payload` | any | No | The actual message data (any JSON type) |

### Requirements

1. **Sequence Numbers**: MUST start at any value and increment by 1 for each message
   - Sequence numbers are tracked per `publisher_id`
   - Restarts/gaps will be detected as missing messages
   - Recommended to persist sequence state across publisher restarts

2. **Publisher IDs**: MUST be unique and stable
   - Use stable identifiers (hostname, UUID, etc.)
   - Do not use random IDs that change on restart
   - Format: alphanumeric with hyphens/underscores recommended

3. **Timestamps**: SHOULD use monotonic time source
   - Not currently used for detection but logged for analysis
   - Helps correlate with external events

## Configuration

Add to `zenohd` configuration (JSON5 format):

```json5
{
  plugins: {
    sequence_detector: {
      // Required: Key expression pattern to monitor
      selector: "sensors/*/data",

      // Optional: How often to log statistics (seconds)
      // Default: 60
      stats_interval_secs: 30,

      // Optional: Key where stats will be published as JSON
      // If omitted, stats are only logged
      stats_key: "admin/sequence_stats"
    }
  }
}
```

### Configuration Options

- **selector**: Key expression pattern to subscribe to (supports wildcards `*` and `**`)
- **stats_interval_secs**: Interval for periodic statistics logging and publication
- **stats_key**: Optional key to publish aggregated statistics

## Usage Examples

### Example 1: Python Publisher with Sequence Tracking

```python
import zenoh
import json
import time

session = zenoh.open()
publisher_id = "sensor-001"
seq = 1

while True:
    msg = {
        "seq": seq,
        "publisher_id": publisher_id,
        "timestamp_ns": time.time_ns(),
        "payload": {
            "temperature": 22.5,
            "reading_id": seq
        }
    }

    session.put("sensors/temperature/data", json.dumps(msg))
    seq += 1
    time.sleep(1)
```

### Example 2: Rust Publisher with Sequence Tracking

```rust
use serde_json::json;
use zenoh::prelude::*;

#[tokio::main]
async fn main() {
    let session = zenoh::open(zenoh::Config::default()).await.unwrap();
    let publisher = session.declare_publisher("sensors/pressure/data").await.unwrap();

    let publisher_id = "sensor-002";
    let mut seq: u64 = 1;

    loop {
        let msg = json!({
            "seq": seq,
            "publisher_id": publisher_id,
            "timestamp_ns": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
            "payload": {
                "pressure_kpa": 101.325,
                "sensor_temp": 21.0
            }
        });

        publisher.put(msg.to_string()).await.unwrap();
        seq += 1;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
```

### Example 3: Monitoring Plugin Output

The plugin logs anomalies to the Zenoh logger:

```
[INFO] Sequence Detector: subscribing to sensors/**/data
[INFO] [sensors/temp/data] First message from publisher 'sensor-001' with seq=1
[DEBUG] [sensors/temp/data] In-sequence message from 'sensor-001': seq=2
[WARN] [sensors/temp/data] OUT-OF-ORDER from 'sensor-001': expected seq=5, received seq=3
[ERROR] [sensors/temp/data] MISSING MESSAGES from 'sensor-001': expected seq=6, received seq=10, gap=4
[WARN] [sensors/temp/data] DUPLICATE from 'sensor-001': seq=10 (already received)
```

### Example 4: Statistics Subscriber

If `stats_key` is configured, statistics are published as JSON:

```python
import zenoh
import json

session = zenoh.open()
subscriber = session.declare_subscriber("admin/sequence_stats")

for sample in subscriber:
    stats = json.loads(sample.payload.to_string())
    for publisher_id, metrics in stats.items():
        print(f"{publisher_id}: {metrics['message_count']} msgs, "
              f"{metrics['missing_count']} missing, "
              f"{metrics['out_of_order_count']} out-of-order")
```

## Detection Logic

### In-Sequence
- Received `seq` = `last_seq + 1`
- Normal operation, logged at DEBUG level

### Missing Messages
- Received `seq` > `last_seq + 1`
- Gap size = `seq - last_seq - 1`
- Logged at ERROR level with gap size

### Out-of-Order
- Received `seq` < `last_seq` and `seq` != `last_seq`
- Message arrived late (previous sequence already received)
- Logged at WARN level

### Duplicate
- Received `seq` = `last_seq`
- Exact duplicate of last message
- Logged at WARN level

## Statistics Schema

Published statistics JSON format:

```json
{
  "sensor-001": {
    "message_count": 1500,
    "last_seq": 1500,
    "out_of_order_count": 3,
    "missing_count": 7
  },
  "sensor-002": {
    "message_count": 2000,
    "last_seq": 2000,
    "out_of_order_count": 0,
    "missing_count": 0
  }
}
```

## Building

From the Zenoh repository root:

```bash
cargo build --release -p zenoh-plugin-sequence-detector
```

The plugin will be built as:
- Linux: `target/release/libzenoh_plugin_sequence_detector.so`
- macOS: `target/release/libzenoh_plugin_sequence_detector.dylib`
- Windows: `target/release/zenoh_plugin_sequence_detector.dll`

## Installation

1. Copy the built library to your plugins directory
2. Add configuration to your `zenohd` config file
3. Restart `zenohd`

The plugin will auto-load if placed in the default plugins directory or specified via `--plugin-search-dir`.

## Performance Considerations

- **Memory**: Tracks O(n) state per unique `publisher_id` (minimal per-publisher overhead)
- **CPU**: JSON parsing per message (consider binary format for high-throughput)
- **Network**: Subscribes to all matching messages (use specific selectors)

For high-throughput scenarios (>10k msgs/sec), consider:
- More specific selectors to reduce message volume
- Binary encoding instead of JSON
- Sampling (modify plugin to track every Nth message)

## Limitations

- Sequence numbers must start from any value and increment by 1
- No automatic recovery or reordering of messages
- Statistics are in-memory only (not persisted across restarts)
- JSON parsing overhead for each message
- No support for compressed or encrypted payloads

## Troubleshooting

### Plugin not loading
- Check `zenohd` logs for plugin loading errors
- Verify plugin library is in search path
- Ensure configuration uses correct plugin name: `sequence_detector`

### No messages detected
- Verify selector matches your publisher's keys
- Check message format matches JSON schema exactly
- Enable DEBUG logging to see all received messages

### False positives
- Ensure `publisher_id` is stable across restarts
- Verify sequence numbers are truly monotonic
- Check for multiple publishers using same `publisher_id`

## Future Enhancements

Potential improvements for future versions:

- Binary message format support for performance
- Configurable tolerance for sequence gaps
- Historical gap detection (late arrivals within window)
- Prometheus metrics export
- Alert webhooks or pub/sub notifications
- Sequence state persistence
- Per-key pattern statistics aggregation

## License

This plugin is part of Eclipse Zenoh and is licensed under:
- Eclipse Public License 2.0 (EPL-2.0)
- Apache License 2.0

See LICENSE file for details.
