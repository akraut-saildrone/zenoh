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

//! Example publisher with sequence tracking
//!
//! This example demonstrates how to publish messages with the sequence
//! tracking format required by the zenoh-plugin-sequence-detector.
//!
//! Usage:
//!   cargo run --example publisher -- [options]
//!
//! Options:
//!   --key <KEY>           Key expression to publish on (default: "sequenced/demo")
//!   --publisher-id <ID>   Publisher identifier (default: hostname-based)
//!   --interval <MS>       Interval between messages in ms (default: 1000)
//!   --missing <N>         Simulate missing message every N messages (0 = none)
//!   --duplicate <N>       Simulate duplicate every N messages (0 = none)
//!   --reorder <N>         Simulate reorder every N messages (0 = none)

use clap::Parser;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use zenoh::config::Config;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Key expression to publish on
    #[arg(long, default_value = "sequenced/demo")]
    key: String,

    /// Publisher identifier (unique ID for this publisher)
    #[arg(long)]
    publisher_id: Option<String>,

    /// Interval between messages in milliseconds
    #[arg(long, default_value = "1000")]
    interval: u64,

    /// Simulate missing message every N messages (0 = none)
    #[arg(long, default_value = "0")]
    missing: u64,

    /// Simulate duplicate every N messages (0 = none)
    #[arg(long, default_value = "0")]
    duplicate: u64,

    /// Simulate out-of-order every N messages (0 = none)
    #[arg(long, default_value = "0")]
    reorder: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Initialize logging
    tracing_subscriber::fmt::init();

    // Generate default publisher_id if not provided
    let publisher_id = args.publisher_id.unwrap_or_else(|| {
        format!(
            "{}-demo-{}",
            hostname::get()
                .unwrap_or_else(|_| "unknown".into())
                .to_string_lossy(),
            std::process::id()
        )
    });

    println!("Starting publisher:");
    println!("  Publisher ID: {}", publisher_id);
    println!("  Key: {}", args.key);
    println!("  Interval: {}ms", args.interval);
    if args.missing > 0 {
        println!("  Simulating missing messages every {} msgs", args.missing);
    }
    if args.duplicate > 0 {
        println!("  Simulating duplicates every {} msgs", args.duplicate);
    }
    if args.reorder > 0 {
        println!("  Simulating reorders every {} msgs", args.reorder);
    }
    println!();

    // Create Zenoh session
    let session = zenoh::open(Config::default()).await.unwrap();
    let publisher = session.declare_publisher(&args.key).await.unwrap();

    let mut seq: u64 = 1;
    let mut pending_reorder: Option<u64> = None;

    loop {
        // Check for reorder simulation
        if args.reorder > 0 && seq % args.reorder == 0 && pending_reorder.is_none() {
            // Store current seq for later, skip this one
            pending_reorder = Some(seq);
            println!("[REORDER] Holding seq={} for next iteration", seq);
            seq += 1;
            continue;
        }

        // Determine which sequence to send
        let send_seq = if let Some(held_seq) = pending_reorder {
            // Send the held message (out of order)
            pending_reorder = None;
            println!("[REORDER] Sending held seq={} after seq={}", held_seq, seq - 1);
            held_seq
        } else {
            seq
        };

        // Check for missing message simulation
        if args.missing > 0 && send_seq % args.missing == 0 {
            println!("[MISSING] Skipping seq={}", send_seq);
            seq += 1;
            tokio::time::sleep(tokio::time::Duration::from_millis(args.interval)).await;
            continue;
        }

        // Create message
        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let message = json!({
            "seq": send_seq,
            "publisher_id": publisher_id,
            "timestamp_ns": timestamp_ns,
            "payload": {
                "counter": send_seq,
                "message": format!("Message number {}", send_seq),
                "timestamp_iso": chrono::Utc::now().to_rfc3339(),
            }
        });

        // Publish message
        let json_str = message.to_string();
        publisher.put(json_str.clone()).await.unwrap();
        println!("[SENT] seq={}", send_seq);

        // Check for duplicate simulation
        if args.duplicate > 0 && send_seq % args.duplicate == 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(args.interval / 2)).await;
            publisher.put(json_str).await.unwrap();
            println!("[DUPLICATE] Re-sent seq={}", send_seq);
        }

        // Increment sequence for next normal message
        if pending_reorder.is_none() {
            seq += 1;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(args.interval)).await;
    }
}
