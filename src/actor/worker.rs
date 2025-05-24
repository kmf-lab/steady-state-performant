use steady_state::*;

// over designed this enum is. much to learn here we have.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[repr(u64)] // Pack everything into 8 bytes
pub(crate) enum FizzBuzzMessage {
    #[default]
    FizzBuzz = 15,         // Discriminant is 15 - could have been any valid FizzBuzz
    Fizz = 3,              // Discriminant is 3 - could have been any valid Fizz
    Buzz = 5,              // Discriminant is 5 - could have been any valid Buzz
    Value(u64),            // Store u64 directly, use the fact that FizzBuzz/Fizz/Buzz only occupy small values
}

impl FizzBuzzMessage {
    pub fn new(value: u64) -> Self {
        match (value % 3, value % 5) {
            (0, 0) => FizzBuzzMessage::FizzBuzz,    // Multiple of 15
            (0, _) => FizzBuzzMessage::Fizz,        // Multiple of 3, not 5
            (_, 0) => FizzBuzzMessage::Buzz,        // Multiple of 5, not 3
            _      => FizzBuzzMessage::Value(value), // Neither
        }
    }
}

pub(crate) struct WorkerState {
    pub(crate) heartbeats_processed: u64,
    pub(crate) values_processed: u64,
    pub(crate) messages_sent: u64,
    pub(crate) batch_size: usize,
}

pub async fn run(context: SteadyContext
                 , heartbeat: SteadyRx<u64>
                 , generator: SteadyRx<u64>
                 , logger: SteadyTx<FizzBuzzMessage>
                 , state: SteadyState<WorkerState>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor([&heartbeat, &generator], [&logger]);
    if cmd.use_internal_behavior {
        internal_behavior(cmd, heartbeat, generator, logger, state).await
    } else {
        cmd.simulated_behavior(vec!(&logger)).await
    }
}

async fn internal_behavior<C: SteadyCommander>(mut cmd: C
                                               , heartbeat: SteadyRx<u64>
                                               , generator: SteadyRx<u64>
                                               , logger: SteadyTx<FizzBuzzMessage>
                                               , state: SteadyState<WorkerState>) -> Result<(),Box<dyn Error>> {

    let mut state = state.lock(|| WorkerState {
        heartbeats_processed: 0,
        values_processed: 0,
        messages_sent: 0,
        batch_size: 512, // Large batch processing
    }).await;

    let mut heartbeat = heartbeat.lock().await;
    let mut generator = generator.lock().await;
    let mut logger = logger.lock().await;

    // Pre-allocate buffers for high-performance batch processing
    let mut generator_batch = vec![0u64; state.batch_size];
    let mut fizzbuzz_batch = Vec::with_capacity(state.batch_size);

    while cmd.is_running(|| i!(heartbeat.is_closed_and_empty()) && i!(generator.is_closed_and_empty()) && i!(logger.mark_closed())) {
        // Wait for heartbeat signal and data availability
        await_for_all!(cmd.wait_avail(&mut heartbeat, 1),
                       cmd.wait_avail(&mut generator, 1));

        // Process heartbeat signals in batches
        while !cmd.is_empty(&mut heartbeat) {
            if let Some(_h) = cmd.try_take(&mut heartbeat) {
                state.heartbeats_processed += 1;

                // Process all available generator data in large batches
                loop {
                    let available = cmd.avail_units(&mut generator);
                    if available == 0 {
                        break;
                    }

                    // Take a slice of generator values
                    let batch_size = available.min(state.batch_size);
                    let taken = cmd.take_slice(&mut generator, &mut generator_batch[..batch_size]);

                    if taken == 0 {
                        break;
                    }

                    // Process the entire batch into FizzBuzz messages
                    fizzbuzz_batch.clear();
                    for &value in &generator_batch[..taken] {
                        fizzbuzz_batch.push(FizzBuzzMessage::new(value));
                    }

                    // Send the entire batch to logger
                    let sent_count = cmd.send_slice_until_full(&mut logger, &fizzbuzz_batch);

                    state.values_processed += taken as u64;
                    state.messages_sent += sent_count as u64;

                    // If we couldn't send all messages, we need to wait for room
                    if sent_count < fizzbuzz_batch.len() {
                        await_for_all!(cmd.wait_vacant(&mut logger, fizzbuzz_batch.len() - sent_count));
                        let remaining_sent = cmd.send_slice_until_full(&mut logger, &fizzbuzz_batch[sent_count..]);
                        state.messages_sent += remaining_sent as u64;
                    }

                    // Log performance metrics periodically
                    if state.values_processed % 10000 == 0 {
                        trace!("Worker: processed {} values, sent {} messages", 
                               state.values_processed, state.messages_sent);
                    }
                }
            }
        }
    }

    info!("Worker shutting down. Heartbeats: {}, Values: {}, Messages: {}", 
          state.heartbeats_processed, state.values_processed, state.messages_sent);
    Ok(())
}

#[cfg(test)]
pub(crate) mod worker_tests {
    use std::thread::sleep;
    use steady_state::*;
    use super::*;

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
                   , &mut Threading::Spawn
            );

        // Send larger batches for performance testing
        let values: Vec<u64> = (0..1000).collect();
        generate_tx.testing_send_all(values, true);
        heartbeat_tx.testing_send_all(vec![0], true);
        graph.start();

        sleep(Duration::from_millis(200));

        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(1))?;

        let results: Vec<FizzBuzzMessage> = logger_rx.testing_take_all();
        assert!(results.len() >= 1000);
        assert_eq!(results[0], FizzBuzzMessage::FizzBuzz);
        assert_eq!(results[1], FizzBuzzMessage::Value(1));
        Ok(())
    }
}
