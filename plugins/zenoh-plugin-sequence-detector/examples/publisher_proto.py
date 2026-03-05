#!/usr/bin/env python3
#
# Copyright (c) 2026 ZettaScale Technology
#
# This program and the accompanying materials are made available under the
# terms of the Eclipse Public License 2.0 which is available at
# http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
# which is available at https://www.apache.org/licenses/LICENSE-2.0.
#
# SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
#
# Contributors:
#   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
#

"""
Example Python publisher with sequence tracking using Protocol Buffers.

This example demonstrates how to publish messages with the sequence
tracking format using protobuf for better performance.

Usage:
    python3 publisher_proto.py [options]

Options:
    --key <KEY>           Key expression to publish on (default: "sequenced/demo")
    --publisher-id <ID>   Publisher identifier (default: hostname-based)
    --interval <SEC>      Interval between messages in seconds (default: 1.0)
    --missing <N>         Simulate missing message every N messages (0 = none)
    --duplicate <N>       Simulate duplicate every N messages (0 = none)
    --reorder <N>         Simulate reorder every N messages (0 = none)

Requirements:
    pip install zenoh protobuf

To generate the protobuf Python code:
    protoc --python_out=. -I../proto ../proto/sequenced_message.proto
"""

import argparse
import socket
import time
from typing import Optional

try:
    import zenoh
except ImportError:
    print("Error: zenoh-python not installed")
    print("Install with: pip install zenoh")
    exit(1)

try:
    import sequenced_message_pb2
except ImportError:
    print("Error: protobuf message not compiled")
    print("Run: protoc --python_out=. -I../proto ../proto/sequenced_message.proto")
    print("Or install with: pip install protobuf && protoc ...")
    exit(1)


def get_default_publisher_id() -> str:
    """Generate a default publisher ID based on hostname."""
    hostname = socket.gethostname()
    return f"{hostname}-proto-demo-{id(object())}"


def create_message(seq: int, publisher_id: str, payload: bytes) -> bytes:
    """Create a sequence-tracked message in protobuf format."""
    message = sequenced_message_pb2.SequencedMessage()
    message.seq = seq
    message.publisher_id = publisher_id
    message.timestamp_ns = time.time_ns()
    message.payload = payload

    return message.SerializeToString()


def main():
    parser = argparse.ArgumentParser(
        description="Sequence-tracked message publisher for Zenoh (Protobuf)"
    )
    parser.add_argument(
        "--key",
        default="sequenced/demo",
        help="Key expression to publish on (default: sequenced/demo)"
    )
    parser.add_argument(
        "--publisher-id",
        help="Publisher identifier (default: hostname-based)"
    )
    parser.add_argument(
        "--interval",
        type=float,
        default=1.0,
        help="Interval between messages in seconds (default: 1.0)"
    )
    parser.add_argument(
        "--missing",
        type=int,
        default=0,
        help="Simulate missing message every N messages (0 = none)"
    )
    parser.add_argument(
        "--duplicate",
        type=int,
        default=0,
        help="Simulate duplicate every N messages (0 = none)"
    )
    parser.add_argument(
        "--reorder",
        type=int,
        default=0,
        help="Simulate out-of-order every N messages (0 = none)"
    )

    args = parser.parse_args()

    # Generate default publisher_id if not provided
    publisher_id = args.publisher_id or get_default_publisher_id()

    print("Starting protobuf publisher:")
    print(f"  Publisher ID: {publisher_id}")
    print(f"  Key: {args.key}")
    print(f"  Interval: {args.interval}s")
    print(f"  Format: Protocol Buffers (binary)")
    if args.missing > 0:
        print(f"  Simulating missing messages every {args.missing} msgs")
    if args.duplicate > 0:
        print(f"  Simulating duplicates every {args.duplicate} msgs")
    if args.reorder > 0:
        print(f"  Simulating reorders every {args.reorder} msgs")
    print()

    # Create Zenoh session
    config = zenoh.Config()
    session = zenoh.open(config)

    seq = 1
    pending_reorder: Optional[int] = None

    try:
        while True:
            # Check for reorder simulation
            if args.reorder > 0 and seq % args.reorder == 0 and pending_reorder is None:
                # Store current seq for later, skip this one
                pending_reorder = seq
                print(f"[REORDER] Holding seq={seq} for next iteration")
                seq += 1
                continue

            # Determine which sequence to send
            if pending_reorder is not None:
                # Send the held message (out of order)
                send_seq = pending_reorder
                pending_reorder = None
                print(f"[REORDER] Sending held seq={send_seq} after seq={seq - 1}")
            else:
                send_seq = seq

            # Check for missing message simulation
            if args.missing > 0 and send_seq % args.missing == 0:
                print(f"[MISSING] Skipping seq={send_seq}")
                seq += 1
                time.sleep(args.interval)
                continue

            # Create payload (simple binary data)
            payload = f"Message number {send_seq}".encode('utf-8')

            # Create and publish message
            message_bytes = create_message(send_seq, publisher_id, payload)
            session.put(args.key, message_bytes)
            print(f"[SENT] seq={send_seq} ({len(message_bytes)} bytes)")

            # Check for duplicate simulation
            if args.duplicate > 0 and send_seq % args.duplicate == 0:
                time.sleep(args.interval / 2)
                session.put(args.key, message_bytes)
                print(f"[DUPLICATE] Re-sent seq={send_seq}")

            # Increment sequence for next normal message
            if pending_reorder is None:
                seq += 1

            time.sleep(args.interval)

    except KeyboardInterrupt:
        print("\nShutting down...")
    finally:
        session.close()


if __name__ == "__main__":
    main()
