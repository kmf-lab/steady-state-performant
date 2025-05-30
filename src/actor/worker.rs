use steady_state::*;

// Keep the clean enum design
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
    pub fn new(value: u64) -> Self {
        match value % 15 {
            0  => FizzBuzzMessage::FizzBuzz, // divisible by both 3 and 5
            3  => FizzBuzzMessage::Fizz,     // divisible by 3
            5  => FizzBuzzMessage::Buzz,     // divisible by 5
            6  => FizzBuzzMessage::Fizz,     // divisible by 3
            9  => FizzBuzzMessage::Fizz,     // divisible by 3
            10 => FizzBuzzMessage::Buzz,     // divisible by 5
            12 => FizzBuzzMessage::Fizz,     // divisible by 3
            _  => FizzBuzzMessage::Value(value), // 1,2,4,7,8,11,13,14
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
    //this is NOT on the edge of the graph so we do not want to simulate it as it will be tested by its simulated neighbors
    internal_behavior(context.into_monitor([&heartbeat, &generator], [&logger]), heartbeat, generator, logger, state).await
}

async fn internal_behavior<C: SteadyCommander>(mut cmd: C
                                               , heartbeat: SteadyRx<u64>
                                               , generator: SteadyRx<u64>
                                               , logger: SteadyTx<FizzBuzzMessage>
                                               , state: SteadyState<WorkerState>) -> Result<(),Box<dyn Error>> {


    let mut heartbeat = heartbeat.lock().await;
    let mut generator = generator.lock().await;
    let mut logger = logger.lock().await;
    const SLICES:usize = 2; // important for high volume throughput
    
    let mut state = state.lock(|| WorkerState {
        heartbeats_processed: 0,
        values_processed: 0,
        messages_sent: 0,
        batch_size: generator.capacity()/SLICES,
    }).await;


    // Pre-allocate buffers for batch processing - this was correct!
    let mut generator_batch = vec![0u64; state.batch_size];
    let mut fizzbuzz_batch = Vec::with_capacity(state.batch_size);

    while cmd.is_running(|| i!(heartbeat.is_closed_and_empty()) && i!(generator.is_closed_and_empty()) && i!(logger.mark_closed())) {

         let is_clean = await_for_all_or_proceed_upon!(
             cmd.wait_periodic(Duration::from_millis(10)),
             cmd.wait_avail(&mut heartbeat, 1),
             cmd.wait_avail(&mut generator, state.batch_size),    // Wait for substantial generator data
             cmd.wait_vacant(&mut logger, state.batch_size)       // Ensure we can send processed results
         );

        let mut slices = SLICES;
        while slices>0 {
            slices -= 1;
            if cmd.try_take(&mut heartbeat).is_some() || !is_clean {
                state.heartbeats_processed += 1;

                // Process ALL available generator data efficiently
                let available = cmd.avail_units(&mut generator).min(cmd.vacant_units(&mut logger));
                if available > 0 {
                    let batch_size = available.min(state.batch_size);
                    // important for high volume throughput
                    let taken = cmd.take_slice(&mut generator, &mut generator_batch[..batch_size]);
                    if taken > 0 {
                        // Convert to FizzBuzz messages efficiently
                        fizzbuzz_batch.clear();
                        fizzbuzz_batch.reserve(taken);
                        for &value in &generator_batch[..taken] {
                            fizzbuzz_batch.push(FizzBuzzMessage::new(value));
                        }

                        // Send batch efficiently using send_slice_until_full
                        let sent_count = cmd.send_slice_until_full(&mut logger, &fizzbuzz_batch);
                        state.values_processed += taken as u64;
                        state.messages_sent += sent_count as u64;
                        assert_eq!(sent_count, fizzbuzz_batch.len(), "expected to match since pre-checked");

                        // Performance logging
                        if state.values_processed % 10_000 == 0 {
                            trace!("Worker processed {} values, sent {} messages",
                                   state.values_processed, state.messages_sent);
                        }
                    }
                }

                if state.heartbeats_processed % 1_000 == 0 {
                    trace!("Worker: {} heartbeats processed", state.heartbeats_processed);
                }
            } else {
                break;
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
