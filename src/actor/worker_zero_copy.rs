use steady_state::*;
use crate::actor::worker_double_buffer::FizzBuzzMessage;

/// State struct for the worker actor.
/// Tracks the number of heartbeats, values processed, messages sent, and the batch size.
/// The batch_size is set to half the channel capacity for double-buffering.
pub(crate) struct WorkerState {
    pub(crate) heartbeats_processed: u64,
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
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    heartbeat: SteadyRx<u64>,
    generator: SteadyRx<u64>,
    logger: SteadyTx<FizzBuzzMessage>,
    state: SteadyState<WorkerState>,
) -> Result<(), Box<dyn Error>> {
    let mut logger = logger.lock().await;
    let mut heartbeat = heartbeat.lock().await;
    let mut generator = generator.lock().await;

    // Initialize the actor's state, setting batch_size to half the generator channel's capacity.
    // This ensures that the producer can fill one half while the consumer processes the other.
    let mut state = state.lock(|| WorkerState {
        heartbeats_processed: 0,
    }).await;

    let min_generator_wait = generator.capacity()/2;
    let min_logger_wait = logger.capacity()/2;
    let max_latency = Duration::from_millis(30);

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
        let is_clean = await_for_all_or_proceed_upon!(
            actor.wait_periodic(max_latency),
            actor.wait_avail(&mut heartbeat, 1),
            actor.wait_avail(&mut generator, min_generator_wait),//#!#//
            actor.wait_vacant(&mut logger, min_logger_wait)
        );


            // Only proceed if a heartbeat is available or if any awaited condition is ready.
            // This ensures we don't leave data stranded in the channel.
            if actor.try_take(&mut heartbeat).is_some() || !is_clean {
                state.heartbeats_processed += 1;


                let (peek_a, peek_b) = actor.peek_slice(&mut generator);//#!#//
                let (poke_a, poke_b) = actor.poke_slice(&mut logger);//#!#//

                let take_count = (peek_a.len() + peek_b.len()).min(poke_a.len() + poke_b.len());
                let switch_input = peek_a.len();
                let switch_output = poke_a.len();

                // Loop 1: peek_a to poke_a
                let end1 = switch_input.min(switch_output).min(take_count);
                for i in 0..end1 {
                    poke_a[i].write(FizzBuzzMessage::new(peek_a[i]));
                }

                // Loop 2: peek_a to poke_b
                let start2 = switch_output;
                let end2 = if switch_output < switch_input {
                    switch_input.min(take_count)
                } else {
                    start2 // Loop is empty
                };
                for i in start2..end2 {
                    poke_b[i - switch_output].write(FizzBuzzMessage::new(peek_a[i]));
                }

                // Loop 3: peek_b to poke_a
                let start3 = switch_input;
                let end3 = if switch_input < switch_output {
                    switch_output.min(take_count)
                } else {
                    start3 // Loop is empty
                };
                for i in start3..end3 {
                    poke_a[i].write(FizzBuzzMessage::new(peek_b[i - switch_input]));
                }

                // Loop 4: peek_b to poke_b
                let start4 = switch_input.max(switch_output);
                let end4 = take_count;
                for i in start4..end4 {
                    poke_b[i - switch_output].write(FizzBuzzMessage::new(peek_b[i - switch_input]));
                }
                
                
                assert_eq!(take_count, actor.advance_send_index(&mut logger, take_count).item_count(), "move write position");//#!#//
                assert_eq!(take_count, actor.advance_take_index(&mut generator, take_count).item_count(), "move read position");//#!#//


                state.heartbeats_processed += take_count as u64;

                if state.heartbeats_processed & ((1<<26)-1) == 0 {
                    trace!("Worker: {} heartbeats processed", state.heartbeats_processed);
                }
            } else {
                // If no heartbeat and no other condition is ready, break out of the double-buffer loop.
                break;
            }
       
    }

    // Final shutdown log, reporting all statistics.
    info!("Worker shutting down. Heartbeats: {}",
          state.heartbeats_processed);
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
