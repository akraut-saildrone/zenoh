# Sequence Detector Message Format Specification

Version: 1.0
Date: 2026-03-04

## Overview

This document defines the message format required for the Zenoh Sequence Detector Plugin to track and detect out-of-sequence and missing messages in Zenoh publish/subscribe streams.

## Format Requirements

### Mandatory Format: JSON

The plugin currently supports **JSON-encoded messages only**. Each message MUST be a valid JSON object with the following structure:

```json
{
  "seq": <sequence_number>,
  "publisher_id": "<unique_publisher_identifier>",
  "timestamp_ns": <optional_timestamp>,
  "payload": <optional_user_data>
}
```

## Field Specifications

### 1. `seq` (REQUIRED)

**Type**: Unsigned 64-bit integer (u64)
**Range**: 0 to 18,446,744,073,709,551,615
**Constraints**:
- MUST be a non-negative integer
- MUST increment by exactly 1 for each subsequent message from the same publisher
- SHOULD start at 1 (but any starting value is acceptable)
- MUST NOT skip values under normal operation
- MUST be tracked independently per `publisher_id`

**Examples**:
```json
{"seq": 1, ...}      // First message
{"seq": 2, ...}      // Second message
{"seq": 3, ...}      // Third message
```

**Invalid**:
```json
{"seq": -1, ...}     // Negative values not allowed
{"seq": 1.5, ...}    // Floating point not allowed
{"seq": "1", ...}    // String not allowed (must be number)
{"seq": null, ...}   // Null not allowed
// Missing "seq"     // Field is required
```

**Sequence Wrap-Around**:
After reaching the maximum u64 value, sequence numbers should wrap to 0. The plugin will detect this as missing messages. For extremely long-running publishers, consider implementing sequence reset notifications.

### 2. `publisher_id` (REQUIRED)

**Type**: String
**Max Length**: 256 characters (recommended: 32-64 characters)
**Constraints**:
- MUST be unique across all publishers in the monitored system
- MUST remain stable across publisher restarts
- SHOULD be human-readable for debugging
- RECOMMENDED: Use a combination of hostname and process identifier
- MUST NOT be empty string

**Recommended Formats**:
```
<hostname>-<process>-<instance>
<uuid>
<mac_address>-<process_name>
<service_name>-<datacenter>-<instance_id>
```

**Examples**:
```json
{"publisher_id": "sensor-001", ...}
{"publisher_id": "server-nyc-web-1", ...}
{"publisher_id": "550e8400-e29b-41d4-a716-446655440000", ...}
{"publisher_id": "iot-device-AA:BB:CC:DD:EE:FF", ...}
```

**Invalid**:
```json
{"publisher_id": "", ...}           // Empty not allowed
{"publisher_id": null, ...}         // Null not allowed
{"publisher_id": 123, ...}          // Must be string
// Missing "publisher_id"           // Field is required
```

**Important**: Using random or session-based IDs that change on restart will cause the plugin to treat each restart as a new publisher, potentially missing sequence gaps across restarts.

### 3. `timestamp_ns` (OPTIONAL)

**Type**: Unsigned 64-bit integer (u64)
**Unit**: Nanoseconds since UNIX epoch (January 1, 1970, 00:00:00 UTC)
**Constraints**:
- SHOULD use system monotonic time when available
- SHOULD be the message creation time, not send time
- Primarily used for logging and correlation (not currently used for detection)

**Example**:
```json
{"timestamp_ns": 1709571234567890123, ...}
```

**Omission**:
```json
// Field can be completely omitted
{"seq": 1, "publisher_id": "sensor-001", "payload": {...}}

// Or explicitly set to null
{"timestamp_ns": null, ...}
```

**Timestamp Precision**:
- Nanoseconds provide microsecond precision for event correlation
- For systems without nanosecond clocks, multiply milliseconds by 1,000,000
- Timezone is irrelevant (always UTC epoch-based)

### 4. `payload` (OPTIONAL)

**Type**: Any valid JSON value (object, array, string, number, boolean, null)
**Constraints**:
- No size limit enforced by plugin (subject to Zenoh message limits)
- Can be omitted entirely if only sequence tracking is needed
- Passed through without modification or validation

**Examples**:

Object payload:
```json
{
  "seq": 1,
  "publisher_id": "sensor-001",
  "payload": {
    "temperature": 22.5,
    "humidity": 65,
    "location": "room-a"
  }
}
```

Array payload:
```json
{
  "seq": 2,
  "publisher_id": "sensor-001",
  "payload": [1, 2, 3, 4, 5]
}
```

Simple value:
```json
{
  "seq": 3,
  "publisher_id": "sensor-001",
  "payload": "status: ok"
}
```

No payload:
```json
{
  "seq": 4,
  "publisher_id": "sensor-001"
}
```

## Complete Examples

### Minimal Valid Message

```json
{
  "seq": 1,
  "publisher_id": "sensor-001"
}
```

### Typical Message

```json
{
  "seq": 42,
  "publisher_id": "weather-station-01",
  "timestamp_ns": 1709571234567890000,
  "payload": {
    "temperature_c": 18.5,
    "pressure_hpa": 1013.25,
    "humidity_pct": 62
  }
}
```

### Message with Complex Payload

```json
{
  "seq": 100,
  "publisher_id": "vehicle-tracker-xyz",
  "timestamp_ns": 1709571234567890000,
  "payload": {
    "vehicle_id": "truck-42",
    "position": {
      "lat": 37.7749,
      "lon": -122.4194,
      "altitude_m": 15
    },
    "speed_kmh": 65.5,
    "heading_deg": 270,
    "sensors": {
      "fuel_level_pct": 75,
      "engine_temp_c": 92,
      "tire_pressure_psi": [32, 32, 31, 33]
    }
  }
}
```

## Validation Rules

The plugin performs the following validation on each received message:

1. **JSON Parsing**: Message must be valid JSON
2. **Required Fields**: Both `seq` and `publisher_id` must be present
3. **Type Checking**: `seq` must be a number, `publisher_id` must be a string
4. **Range Validation**: `seq` must be non-negative

Messages failing validation are logged as warnings and ignored for sequence tracking.

## Message Ordering Guarantees

The Sequence Detector Plugin does **NOT** reorder messages. It only:
- **Detects** out-of-order arrival
- **Reports** sequence gaps
- **Logs** anomalies

Applications requiring strict ordering must implement their own reorder buffers based on the plugin's output.

## Multi-Publisher Scenarios

When multiple publishers send to the same key:

### Option 1: Unique Keys per Publisher (RECOMMENDED)

```
sensors/sensor-001/data
sensors/sensor-002/data
sensors/sensor-003/data
```

Configure plugin with wildcard:
```json5
selector: "sensors/*/data"
```

### Option 2: Shared Key with Publisher ID Discrimination

```
sensors/all/data
```

All publishers send to same key but use unique `publisher_id` values:
```json
// Publisher 1
{"seq": 1, "publisher_id": "sensor-001", "payload": {...}}

// Publisher 2
{"seq": 1, "publisher_id": "sensor-002", "payload": {...}}
```

The plugin tracks sequences independently per `publisher_id`.

## Binary Format (Future Extension)

A binary format may be supported in future versions for performance-critical applications:

```
Offset | Size | Field
-------|------|-------------
0      | 8    | seq (u64, little-endian)
8      | 8    | timestamp_ns (u64, little-endian)
16     | 32   | publisher_id (UTF-8, zero-padded)
48     | N    | payload (variable length binary)
```

This format would reduce overhead for high-frequency messages (>10k msgs/sec).

## Best Practices

### For Publishers

1. **Persistent Sequence State**: Save sequence number to disk/database before publishing
   ```python
   seq = load_sequence_from_storage()
   publish_message(seq, ...)
   save_sequence_to_storage(seq + 1)
   ```

2. **Atomic Increment**: Use atomic operations in multi-threaded publishers
   ```rust
   let seq = sequence_counter.fetch_add(1, Ordering::SeqCst);
   ```

3. **Restart Recovery**: Initialize sequence from last known value + 1
   ```python
   if os.path.exists("seq.txt"):
       with open("seq.txt") as f:
           seq = int(f.read()) + 1
   else:
       seq = 1
   ```

4. **Stable Publisher IDs**: Use configuration files or environment variables
   ```bash
   PUBLISHER_ID="${HOSTNAME}-${SERVICE_NAME}-${INSTANCE_ID}"
   ```

### For Subscribers

1. **Parse the Full Message**: Don't strip the sequence metadata
   ```python
   msg = json.loads(sample.payload.to_string())
   seq = msg["seq"]
   data = msg["payload"]
   ```

2. **Correlate with Plugin Logs**: Match your application logs with plugin warnings
   ```python
   if last_seq + 1 != msg["seq"]:
       logger.warning(f"Gap detected, check sequence-detector logs")
   ```

3. **Monitor Statistics**: Subscribe to stats key if configured
   ```python
   stats_sub = session.declare_subscriber("admin/sequence_stats")
   ```

## Error Handling

### Invalid Message Format

```
[WARN] [key/expr] Failed to parse message as SequencedMessage: missing field `seq`
```

**Action**: Fix publisher to include required fields

### Out of Order

```
[WARN] [key/expr] OUT-OF-ORDER from 'pub-1': expected seq=5, received seq=3
```

**Action**: Investigate network reordering, check for multiple publishers with same ID

### Missing Messages

```
[ERROR] [key/expr] MISSING MESSAGES from 'pub-1': expected seq=6, received seq=10, gap=4
```

**Action**: Check publisher for crashes, network issues, or sequence increment bugs

### Duplicates

```
[WARN] [key/expr] DUPLICATE from 'pub-1': seq=10 (already received)
```

**Action**: Check for retransmission logic, publisher restart without state recovery

## Version History

- **1.0** (2026-03-04): Initial specification
  - JSON format definition
  - Required fields: `seq`, `publisher_id`
  - Optional fields: `timestamp_ns`, `payload`
  - Validation rules
  - Best practices

## Future Considerations

- Binary format for high-throughput scenarios
- Message authentication codes (MACs) for integrity
- Compression support
- Batch message format (multiple sequences in one message)
- Sequence range acknowledgments
