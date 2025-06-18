use steady_state::*;

/// FizzBuzzMessage is the output type for the worker actor.
/// This enum is designed for efficient, branchless pattern matching in batch processing.
/// The #[repr(u64)] ensures a predictable memory layout, which is cache-friendly.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum FizzBuzzMessage {
    #[default]
    FizzBuzz = 15,
    Fizz = 3,
    Buzz = 5,
    Value(u64),
}

impl FizzBuzzMessage {
    /// Efficiently computes the FizzBuzz variant for a given value.
    /// This function is called in tight loops, so it avoids allocations and minimizes branching.
    pub fn new(value: u64) -> Self {
        match value % 15 {
            0  => FizzBuzzMessage::FizzBuzz, // divisible by both 3 and 5
            3  => FizzBuzzMessage::Fizz,     // divisible by 3
            5  => FizzBuzzMessage::Buzz,     // divisible by 5
            6  => FizzBuzzMessage::Fizz,     // divisible by 3
            9  => FizzBuzzMessage::Fizz,     // divisible by 3
            10 => FizzBuzzMessage::Buzz,     // divisible by 5
            12 => FizzBuzzMessage::Fizz,     // divisible by 3
            _  => FizzBuzzMessage::Value(value), // all other values
        }
    }
}

/// State struct for the worker actor.
/// Tracks the number of heartbeats, values processed, messages sent, and the batch size.
/// The batch_size is set to half the channel capacity for double-buffering.
pub(crate) struct WorkerState {
    pub(crate) heartbeats_processed: u64,
    pub(crate) values_processed: u64,
    pub(crate) messages_sent: u64,
    pub(crate) batch_size: usize,
}

/// Entry point for the worker actor.
/// This actor is not on the edge of the graph, so it is always run with real neighbors.
/// It receives heartbeats and generator values, processes them in batches, and sends FizzBuzz messages to the logger.
pub async fn run(
    actor: SteadyActorShadow,
    heartbeat: SteadyRx<u64>,
    generator: SteadyRx<u64>,
    logger: SteadyTx<FizzBuzzMessage>,
    state: SteadyState<WorkerState>,
) -> Result<(), Box<dyn Error>> {
    // The worker is tested by its simulated neighbors, so we always use internal_behavior.
    internal_behavior(
        actor.into_spotlight([&heartbeat, &generator], [&logger]),
        heartbeat,
        generator,
        logger,
        state,
    )
        .await
}

/// The core logic for the worker actor.
/// This function implements high-throughput, cache-friendly batch processing.
///
/// Key performance strategies:
/// - **Double-buffering**: The channel is logically split into two halves. While one half is being filled by the producer, the consumer processes the other half.
/// - **Full-channel consumption**: The worker processes both halves (two slices) before yielding, maximizing cache line reuse and minimizing context switches.
/// - **Pre-allocated buffers**: All batch buffers are allocated once and reused, ensuring zero-allocation hot paths.
/// - **Mechanically sympathetic**: The design aligns with CPU cache and memory bus behavior for optimal throughput.
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    heartbeat: SteadyRx<u64>,
    generator: SteadyRx<u64>,
    logger: SteadyTx<FizzBuzzMessage>,
    state: SteadyState<WorkerState>,
) -> Result<(), Box<dyn Error>> {
    // Lock all channels for exclusive access within this actor.
    let mut heartbeat = heartbeat.lock().await;
    let mut generator = generator.lock().await;
    let mut logger = logger.lock().await;

    // SLICES determines how many times we process a half-batch before yielding.
    // For double-buffering, this is set to 2.
    const SLICES: usize = 2; // important for high volume throughput

    // Initialize the actor's state, setting batch_size to half the generator channel's capacity.
    // This ensures that the producer can fill one half while the consumer processes the other.
    let mut state = state.lock(|| WorkerState {
        heartbeats_processed: 0,
        values_processed: 0,
        messages_sent: 0,
        batch_size: generator.capacity() / SLICES,
    }).await;

    // Pre-allocate buffers for batch processing.
    // generator_batch: holds up to half the channel's values at a time.
    // fizzbuzz_batch: holds the converted FizzBuzz messages for a batch.
    let mut generator_batch = vec![0u64; state.batch_size];
    let mut fizzbuzz_batch = Vec::with_capacity(state.batch_size);

    // Main processing loop.
    // The actor runs until all input channels are closed and empty, and the output channel is closed.
    while actor.is_running(||
        i!(heartbeat.is_closed_and_empty()) &&
            i!(generator.is_closed_and_empty()) &&
            i!(logger.mark_closed())
    ) {
        // Wait for all required conditions:
        // - A periodic timer (to avoid starvation)
        // - At least one heartbeat (to trigger processing)
        // - At least half a channel's worth of generator data (for batch efficiency)
        // - Sufficient space in the logger channel for a batch
        let is_clean = await_for_all_or_proceed_upon!(
            actor.wait_periodic(Duration::from_millis(10)),
            actor.wait_avail(&mut heartbeat, 1),
            actor.wait_avail(&mut generator, state.batch_size),
            actor.wait_vacant(&mut logger, state.batch_size)
        );

        // The double-buffering loop: process two slices (halves) before yielding.
        let mut slices = SLICES;
        while slices > 0 {
            slices -= 1;

            // Only proceed if a heartbeat is available or if any awaited condition is ready.
            // This ensures we don't leave data stranded in the channel.
            if actor.try_take(&mut heartbeat).is_some() || !is_clean {
                state.heartbeats_processed += 1;

                // Determine how many values we can process in this batch.
                // We never process more than batch_size at a time.
                let available = actor.avail_units(&mut generator).min(actor.vacant_units(&mut logger));
                if available > 0 {
                    let batch_size = available.min(state.batch_size);

                    // Take a slice of generator values into the pre-allocated buffer.
                    // This is a zero-allocation, cache-friendly operation.
                    let taken = actor.take_slice(&mut generator, &mut generator_batch[..batch_size]).item_count();
                    if taken > 0 {
                        // Convert the batch of values to FizzBuzz messages.
                        // The fizzbuzz_batch buffer is reused every cycle.
                        fizzbuzz_batch.clear();
                        fizzbuzz_batch.reserve(taken);
                        for &value in &generator_batch[..taken] {
                            fizzbuzz_batch.push(FizzBuzzMessage::new(value));
                        }

                        // Send the entire batch to the logger in one operation.
                        // This minimizes synchronization and maximizes throughput.
                        let sent_count = actor.send_slice(&mut logger, &fizzbuzz_batch).item_count();
                        state.values_processed += taken as u64;
                        state.messages_sent += sent_count as u64;
                        assert_eq!(sent_count, fizzbuzz_batch.len(), "expected to match since pre-checked");

                        // Log performance statistics periodically.
                        if state.values_processed & ((1<<18)-1) == 0 {
                            trace!("Worker processed {} values, sent {} messages",
                                   state.values_processed, state.messages_sent);
                        }
                    }
                }

                // Log heartbeat statistics periodically.
                if state.heartbeats_processed & ((1<<13)-1) == 0 {
                    trace!("Worker: {} heartbeats processed", state.heartbeats_processed);
                }
            } else {
                // If no heartbeat and no other condition is ready, break out of the double-buffer loop.
                break;
            }
        }
    }

    // Final shutdown log, reporting all statistics.
    info!("Worker shutting down. Heartbeats: {}, Values: {}, Messages: {}",
          state.heartbeats_processed, state.values_processed, state.messages_sent);
    Ok(())
}

#[cfg(test)]
pub(crate) mod worker_tests {
    use std::thread::sleep;
    use steady_state::*;
    use super::*;

    /// Unit test for the worker actor.
    /// This test verifies that the worker processes batches correctly and produces the expected FizzBuzz output.
    #[test]
    fn test_worker() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (generate_tx, generate_rx) = graph.channel_builder()
            .with_capacity(2048)
            .build();
        let (heartbeat_tx, heartbeat_rx) = graph.channel_builder()
            .with_capacity(512)
            .build();
        let (logger_tx, logger_rx) = graph.channel_builder()
            .with_capacity(2048)
            .build::<FizzBuzzMessage>();

        let state = new_state();
        graph.actor_builder().with_name("UnitTest")
            .build(move |context| internal_behavior(context
                                                    , heartbeat_rx.clone()
                                                    , generate_rx.clone()
                                                    , logger_tx.clone()
                                                    , state.clone())
                   , SoloAct
            );

        let values: Vec<u64> = (0..1000).collect();
        generate_tx.testing_send_all(values, true);
        heartbeat_tx.testing_send_all(vec![0], true);
        graph.start();

        sleep(Duration::from_millis(200));

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(1))?;

        let results: Vec<FizzBuzzMessage> = logger_rx.testing_take_all();
        assert!(results.len() >= 1000);
        assert_eq!(results[0], FizzBuzzMessage::FizzBuzz);
        assert_eq!(results[1], FizzBuzzMessage::Value(1));
        Ok(())
    }
}
