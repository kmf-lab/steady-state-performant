use steady_state::*;
use crate::actor::worker_double_buffer::FizzBuzzMessage;

pub(crate) struct LoggerState {
    pub(crate) messages_logged: u64,
    pub(crate) fizz_count: u64,
    pub(crate) buzz_count: u64,
    pub(crate) fizzbuzz_count: u64,
    pub(crate) value_count: u64,
}

pub async fn run(actor: SteadyActorShadow, fizz_buzz_rx: SteadyRx<FizzBuzzMessage>, state: SteadyState<LoggerState>) -> Result<(),Box<dyn Error>> {
    let actor = actor.into_spotlight([&fizz_buzz_rx], []);
    if actor.use_internal_behavior {
        internal_behavior(actor, fizz_buzz_rx, state).await
    } else {
        actor.simulated_behavior(vec!(&fizz_buzz_rx)).await
    }
}

async fn internal_behavior<A: SteadyActor>(mut cmd: A, rx: SteadyRx<FizzBuzzMessage>, state: SteadyState<LoggerState>) -> Result<(),Box<dyn Error>> {


    let mut rx = rx.lock().await;
    
    let mut state = state.lock(|| LoggerState {
        messages_logged: 0,
        fizz_count: 0,
        buzz_count: 0,
        fizzbuzz_count: 0,
        value_count: 0,
    }).await;


    let batch_size = rx.capacity()/2;
    let max_latency = Duration::from_millis(30);
    // For double buffering solution
    //let mut batch = vec![FizzBuzzMessage::default(); batch_size];

    while cmd.is_running(|| rx.is_closed_and_empty()) {
        await_for_any!(cmd.wait_periodic(max_latency),
                       cmd.wait_avail(&mut rx, batch_size));


        // //zero copy solution
        let (a,b) = cmd.peek_slice(&mut rx);
        let len = a.len() + b.len();
        consume_items(&mut state, a);
        consume_items(&mut state, b);
        cmd.advance_take_index(&mut rx, len);

        // //double buffer solution
        // let taken = cmd.take_slice(&mut rx, &mut batch).item_count();
        // if taken > 0 {
        //     // Process the entire batch efficiently
        //     let x = &batch[..taken];
        //     consume_items(&mut state, x);
        // }
        
    }

    info!("Logger shutting down. Total: {} (F:{}, B:{}, FB:{}, V:{})",
          state.messages_logged, state.fizz_count, state.buzz_count,
          state.fizzbuzz_count, state.value_count);
    Ok(())
}

fn consume_items(state: &mut StateGuard<LoggerState>, items: &[FizzBuzzMessage]) {


        let mut fizz = 0u64;
        let mut buzz = 0u64;
        let mut fizzbuzz = 0u64;
        let mut value = 0u64;
        let mut total = state.messages_logged;
        for &msg in items {
            match msg {
                FizzBuzzMessage::Fizz => fizz += 1,
                FizzBuzzMessage::Buzz => buzz += 1,
                FizzBuzzMessage::FizzBuzz => fizzbuzz += 1,
                FizzBuzzMessage::Value(_) => value += 1,
            }
            total += 1;
            if total < 16 || (total & ((1 << 27) - 1)) == 0 {
                info!(
                            "Logger: {} messages processed (F:{}, B:{}, FB:{}, V:{})",
                            total,
                            state.fizz_count+fizz,
                            state.buzz_count+buzz,
                            state.fizzbuzz_count+fizzbuzz,
                            state.value_count+value
                        );
            }
        }
        state.fizz_count += fizz;
        state.buzz_count += buzz;
        state.fizzbuzz_count += fizzbuzz;
        state.value_count += value;
        state.messages_logged = total;

}

#[test]
fn test_logger() -> Result<(), Box<dyn std::error::Error>> {
    use steady_logger::*;
    let _guard = start_log_capture();
    use std::thread::sleep;

    let mut graph = GraphBuilder::for_testing().build(());
    let (fizz_buzz_tx, fizz_buzz_rx) = graph.channel_builder()
        .with_capacity(4096) // Large capacity for performance
        .build();

    let state = new_state();
    graph.actor_builder().with_name("UnitTest")
        .build(move |context| {
            internal_behavior(context, fizz_buzz_rx.clone(), state.clone())
        }
               , SoloAct);

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
