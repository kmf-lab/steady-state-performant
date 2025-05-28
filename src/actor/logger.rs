use std::thread::sleep;
use steady_state::*;
use crate::actor::worker::FizzBuzzMessage;

pub(crate) struct LoggerState {
    pub(crate) messages_logged: u64,
    pub(crate) batch_size: usize,
    pub(crate) fizz_count: u64,
    pub(crate) buzz_count: u64,
    pub(crate) fizzbuzz_count: u64,
    pub(crate) value_count: u64,
}

pub async fn run(context: SteadyContext, fizz_buzz_rx: SteadyRx<FizzBuzzMessage>, state: SteadyState<LoggerState>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor([&fizz_buzz_rx], []);
    if cmd.use_internal_behavior {
        internal_behavior(cmd, fizz_buzz_rx, state).await
    } else {
        cmd.simulated_behavior(vec!(&fizz_buzz_rx)).await
    }
}

async fn internal_behavior<C: SteadyCommander>(mut cmd: C, rx: SteadyRx<FizzBuzzMessage>, state: SteadyState<LoggerState>) -> Result<(),Box<dyn Error>> {
    let mut state = state.lock(|| LoggerState {
        messages_logged: 0,
        batch_size: 1024*32, // Large batch processing for maximum throughput
        fizz_count: 0,
        buzz_count: 0,
        fizzbuzz_count: 0,
        value_count: 0,
    }).await;

    let mut rx = rx.lock().await;

    // Pre-allocate batch buffer for high-performance processing
    let mut batch = vec![FizzBuzzMessage::default(); state.batch_size];

    while cmd.is_running(|| i!(rx.is_closed_and_empty())) {
        await_for_all_or_proceed_upon!(cmd.wait_periodic(Duration::from_millis(40)),
                                       cmd.wait_avail(&mut rx, state.batch_size));

        // Process messages in large batches for maximum throughput
        let available = cmd.avail_units(&mut rx);
        if available > 0 {
            let batch_size = available.min(state.batch_size);
            let taken = cmd.take_slice(&mut rx, &mut batch[..batch_size]);

            if taken > 0 {
                // Process entire batch efficiently
                for &msg in &batch[..taken] {
                    match msg {
                        FizzBuzzMessage::Fizz => {
                            state.fizz_count += 1;
                        }
                        FizzBuzzMessage::Buzz => {
                            state.buzz_count += 1;
                        }
                        FizzBuzzMessage::FizzBuzz => {
                            state.fizzbuzz_count += 1;
                        }
                        FizzBuzzMessage::Value(_) => {
                            state.value_count += 1;
                        }
                    }


                    state.messages_logged += 1;

                    // Log performance metrics periodically
                    if state.messages_logged % 10_000_000 == 0 {
                        info!("Logger: {} messages processed (F:{}, B:{}, FB:{}, V:{})",
                              state.messages_logged, state.fizz_count, state.buzz_count,
                              state.fizzbuzz_count, state.value_count);
                    } else if state.messages_logged % 1_000_000 == 0 {
                        trace!("Logger: {} messages processed", state.messages_logged);
                    }
                }
            }
        }
    }

    info!("Logger shutting down. Total: {} (F:{}, B:{}, FB:{}, V:{})",
          state.messages_logged, state.fizz_count, state.buzz_count,
          state.fizzbuzz_count, state.value_count);
    Ok(())
}

#[test]
fn test_logger() -> Result<(), Box<dyn std::error::Error>> {
    use steady_logger::*;
    let _guard = start_log_capture();

    let mut graph = GraphBuilder::for_testing().build(());
    let (fizz_buzz_tx, fizz_buzz_rx) = graph.channel_builder()
        .with_capacity(2048) // Large capacity for performance
        .build();

    let state = new_state();
    graph.actor_builder().with_name("UnitTest")
        .build(move |context| {
            internal_behavior(context, fizz_buzz_rx.clone(), state.clone())
        }
               , &mut Threading::Spawn);

    graph.start();

    // Send large batch for performance testing
    let test_messages: Vec<FizzBuzzMessage> = (0..1000)
        .map(FizzBuzzMessage::new)
        .collect();
    fizz_buzz_tx.testing_send_all(test_messages, true);

    sleep(Duration::from_millis(500));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(1))?;

    // Should see batch processing logs
    assert_in_logs!(["Logger:"]);

    Ok(())
}
