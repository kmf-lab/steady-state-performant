# Steady State Performant

> ** Mechanically Sympathetic, Cache-Friendly High-Throughput Actors**

This project demonstrates advanced, real-world performance engineering with the [`steady_state`](https://github.com/steady-stack/steady-state) actor framework.  
It builds on the `minimum` and `standard` lessons, introducing **cache-friendly batching**, **very large channels**, and **mechanically sympathetic** flow control for enterprise-grade throughput.

---

## 🚀 What Makes This "Performant"?

This lesson introduces three key optimizations for maximum throughput:

1. **Extremely Large Channel Buffers**  
   Channels are sized to **over one million messages** (`1 << 20` = 1,048,576), reducing backpressure and synchronization overhead to near zero.

2. **Double-Buffer Batching**  
   Each actor processes *half* of a channel’s capacity in a single batch (over 500,000 messages at a time), while the other half is being filled by upstream actors.  
   This ensures that producers and consumers are always working in parallel, maximizing CPU and memory bus utilization.

3. **Full-Channel Consumption**  
   The worker actor reads *both halves* of its input channel (a full buffer’s worth, over one million messages) before yielding or checking for shutdown.  
   This minimizes context switches, maximizes cache line reuse, and aligns with hardware prefetching for optimal throughput.

---

## 🏗️ System Architecture

**Pipeline:**

```
Generator (batch: 524,288) → Worker (batch: 524,288 x 2) → Logger (batch: large)
↗
Heartbeat (burst: 32)
```

- **Generator**: Produces values in very large batches, filling half the channel at a time.
- **Heartbeat**: Triggers worker batches in bursts, coordinating timing.
- **Worker**: Consumes a full channel (two half-batches, over a million messages) before yielding, converting values to FizzBuzz messages.
- **Logger**: Processes large batches, tracking statistics and throughput.

---

## 🧠 Performance Concepts

### Why Extremely Large Channels?

- **Reduces contention**: Fewer synchronization points between producers and consumers.
- **Absorbs bursts**: Handles spikes in production or consumption without stalling.
- **Enables massive batching**: Allows actors to process huge, contiguous memory regions, maximizing cache and memory bus efficiency.

### Double-Buffer Batching

- **Producer** fills half the channel while **consumer** processes the other half.
- **Consumer** waits for at least half-full, then processes in a single, cache-friendly loop.
- **After consuming one half**, the consumer immediately checks if the other half is ready, processing both halves before yielding.
- **Result**: Maximizes cache line reuse, minimizes context switches, and keeps both ends of the pipeline busy.

### Mechanically Sympathetic Design

- **Cache line friendly**: Batches are sized to fit L1/L2 cache lines, reducing memory latency.
- **Prefetching**: Large, contiguous reads/writes allow hardware prefetchers to work efficiently.
- **Minimal branching**: Processing is done in tight, predictable loops.

---

## ⚙️ Default Runtime Parameters

The system is designed to run at high speed by default:

- **Heartbeat rate:** 2 ms between operations (`--rate 2`)
- **Beats (iterations):** 30,000 (`--beats 30000`)

This means the system will run for about one minute, processing billions of messages in that time.

---

## 📊 Real-World Telemetry: True Throughput

Below is a real snapshot from the built-in telemetry server, running on a modern i5 CPU.  
This shows the system sustaining **over 64 million messages per second** through the main pipeline, with all actors and channels operating at maximum efficiency.

```
digraph G {
"generator" -> "worker" [label="Window 10.2 secs
Avg rate: 64M per/sec
filled 80%ile 100 %
Capacity: 1048576 Total: 941,231,872
", color=red, penwidth=1];
"worker" -> "logger" [label="Window 10.2 secs
Avg rate: 64M per/sec
filled 80%ile 100 %
Capacity: 1048576 Total: 941,231,872
", color=grey, penwidth=1];
}
```

**What does this show?**
- The generator and worker are moving over **64 million messages per second**.
- Channels are running at 100% fill (full double-buffering).
- CPU load is moderate, showing the design is not just fast, but efficient and scalable.

---

## 🏁 Example Output

After running for about a minute with default settings, you will see output like:

```
Logger: 9613344768 messages processed (F:2563558604, B:1281779302, FB:640889652, V:5127117210)
Logger: 9630121984 messages processed (F:2568032529, B:1284016264, FB:642008133, V:5136065058)
Logger: 9646899200 messages processed (F:2572506453, B:1286253226, FB:643126614, V:5145012907)
Generator shutting down. Total generated: 9652662016
Worker shutting down. Heartbeats: 30001, Values: 9652662016, Messages: 9652662016
Logger shutting down. Total: 9652662016 (F:2574043204, B:1287021602, FB:643510802, V:5148086408)
```

This demonstrates the system processed **over 9.6 billion messages** in about a minute, with all actors and channels operating at full efficiency.

---

## 🏎️ Performance Features

- **Batch Processing**: Over 500,000 messages per operation.
- **Zero-Allocation Hot Paths**: Pre-allocated buffers reused every cycle.
- **Real-Time Metrics**: Throughput, batch efficiency, and memory usage tracked live.
- **Actor Teams**: Lightweight actors share threads for optimal core utilization.
- **Backpressure Management**: Large buffers and batch-aware flow control prevent stalls.

---

## 📈 Throughput Scaling

| Version      | Messages/sec | Batch Size      | Channel Capacity | Memory Usage |
|--------------|-------------|-----------------|------------------|-------------|
| Minimum      | ~1,000      | 1               | 64               | Variable    |
| Standard     | ~10,000     | 16–64           | 1024             | Variable    |
| **Performant** | **64,000,000+** | 524,288–1,048,576 | 1,048,576        | Constant    |

---

## 🛠️ Usage

```bash
# Run with default high-speed settings (2ms heartbeat, 30,000 beats)
cargo run

# Custom rate and beats
cargo run -- --rate 10 --beats 1000

# Performance testing with metrics
RUST_LOG=trace cargo run -- --rate 50 --beats 500
```

---

## 🎯 Key Takeaways

- **Batching** and **very large channels** are essential for high-throughput actor systems.
- **Double-buffering** keeps both producer and consumer busy, maximizing hardware efficiency.
- **Full-channel consumption** aligns with CPU cache and memory bus design for peak performance.
- **Mechanically sympathetic** code is not just fast—it’s robust under real-world load.

---

## 📚 Next Steps

- Try adjusting channel sizes and batch sizes to see their effect on throughput.
- Explore the `steady_state` metrics dashboard for real-time performance insights.
- Compare with the `minimum` and `standard` lessons to see the impact of each optimization.

