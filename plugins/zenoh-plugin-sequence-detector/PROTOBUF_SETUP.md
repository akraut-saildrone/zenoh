# Protocol Buffers Setup Guide

The Zenoh Sequence Detector Plugin supports both JSON and Protocol Buffers message formats. Protobuf provides better performance for high-throughput scenarios.

## Installing Protocol Buffers Compiler

### Ubuntu/Debian
```bash
sudo apt-get update
sudo apt-get install -y protobuf-compiler
```

### macOS (Homebrew)
```bash
brew install protobuf
```

### macOS (MacPorts)
```bash
sudo port install protobuf3-cpp
```

### Fedora/CentOS/RHEL
```bash
sudo dnf install protobuf-compiler
```

### Arch Linux
```bash
sudo pacman -S protobuf
```

### Windows (Chocolatey)
```powershell
choco install protoc
```

### Manual Installation

1. Download from: https://github.com/protocolbuffers/protobuf/releases
2. Extract and add `bin/protoc` to your PATH

### Verify Installation
```bash
protoc --version
# Should output: libprotoc 3.x.x or newer
```

## Building the Plugin

Once `protoc` is installed:

```bash
cd plugins/zenoh-plugin-sequence-detector
cargo build --release
```

The build script will automatically compile the `.proto` files.

## Using Protobuf Messages

### Python

1. Install requirements:
```bash
pip install zenoh protobuf
```

2. Generate Python protobuf code:
```bash
cd examples
protoc --python_out=. -I../proto ../proto/sequenced_message.proto
```

3. Run the protobuf publisher:
```bash
./publisher_proto.py --key sequenced/demo
```

### Rust

Add to your `Cargo.toml`:
```toml
[dependencies]
prost = "0.12"

[build-dependencies]
prost-build = "0.12"
```

Example code:
```rust
use prost::Message;

// Include generated code
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/sequenced_message.rs"));
}

// Create a message
let msg = proto::SequencedMessage {
    seq: 42,
    publisher_id: "sensor-001".to_string(),
    timestamp_ns: Some(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64),
    payload: Some(b"Hello".to_vec()),
};

// Serialize
let bytes = msg.encode_to_vec();

// Publish
publisher.put(bytes).await?;
```

### C++

1. Generate C++ code:
```bash
protoc --cpp_out=. proto/sequenced_message.proto
```

2. Link against `protobuf` library and use the generated `sequenced_message.pb.h`

### Java

1. Generate Java code:
```bash
protoc --java_out=. proto/sequenced_message.proto
```

2. Add protobuf dependency to your build (Maven/Gradle)

## Message Format Comparison

### JSON (Human-readable, ~200 bytes)
```json
{
  "seq": 42,
  "publisher_id": "sensor-001",
  "timestamp_ns": 1709571234567890123,
  "payload": {"temp": 22.5}
}
```

### Protobuf (Binary, ~80 bytes for same data)
```
Binary format, approximately 60% smaller
```

## Performance Benefits

| Format | Size | Parse Speed | Network Overhead |
|--------|------|-------------|------------------|
| JSON   | ~200 bytes | Slower | Higher |
| Protobuf | ~80 bytes | 3-5x faster | 60% reduction |

**Recommendation**: Use Protobuf for:
- High-frequency publishers (>1000 msgs/sec)
- Bandwidth-constrained networks
- Large payloads
- Cross-language compatibility requirements

Use JSON for:
- Debugging and development
- Human-readable logs
- Low-frequency data (<100 msgs/sec)
- Simple integration

## Troubleshooting

### "protoc not found" error

**Symptom**: Build fails with `Could not find protoc`

**Solution**: Install `protobuf-compiler` package (see above)

### Python import error: "No module named 'sequenced_message_pb2'"

**Symptom**: Python script fails to import protobuf module

**Solution**:
```bash
cd examples
protoc --python_out=. -I../proto ../proto/sequenced_message.proto
```

### Rust compile error: "failed to compile protobuf definitions"

**Symptom**: Cargo build fails during protobuf compilation

**Solutions**:
1. Ensure `protoc` is in PATH: `which protoc`
2. Set PROTOC environment variable: `export PROTOC=/path/to/protoc`
3. Reinstall protobuf compiler

### Version mismatch errors

**Symptom**: Runtime errors about incompatible protobuf versions

**Solution**: Ensure consistent versions:
```bash
# Check protoc version
protoc --version

# Update Cargo.toml to match
prost = "0.12"  # Should match protoc version family
```

## Additional Resources

- Protocol Buffers Documentation: https://protobuf.dev/
- prost (Rust): https://docs.rs/prost/
- protobuf (Python): https://pypi.org/project/protobuf/
- Zenoh Documentation: https://zenoh.io/docs/
