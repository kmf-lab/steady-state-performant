use steady_state::*;

pub(crate) struct GeneratorState {
    pub(crate) total_generated: u64,
}

pub async fn run(actor: SteadyActorShadow, generated_tx: SteadyTx<u64>, state: SteadyState<GeneratorState>) -> Result<(),Box<dyn Error>> {
    let actor = actor.into_spotlight([], [&generated_tx]);
    if actor.use_internal_behavior {
        internal_behavior(actor, generated_tx, state).await
    } else {
        actor.simulated_behavior(vec!(&generated_tx)).await
    }
}

async fn internal_behavior<A: SteadyActor>(mut actor: A, generated: SteadyTx<u64>, state: SteadyState<GeneratorState> ) -> Result<(),Box<dyn Error>> {

    let mut generated = generated.lock().await;

    let mut state = state.lock(|| GeneratorState {
        total_generated: 0,
    }).await;
    let wait_for= generated.capacity()/4;
    let mut next_value = state.total_generated;

    while actor.is_running(|| i!(generated.mark_closed())) {
        // Wait for sufficient room in channel for our batch
        await_for_all!(actor.wait_vacant(&mut generated, wait_for));
                
        let (poke_a,poke_b) = actor.poke_slice(&mut generated);
        let count = poke_a.len() + poke_b.len();
        for i in 0..poke_a.len() {
            poke_a[i].write(next_value);
            next_value += 1;
        } 
        for i in 0..poke_b.len() {
            poke_b[i].write(next_value);
            next_value += 1;
        }            
       
        let sent_count = actor.advance_send_index(&mut generated, count).item_count();
        state.total_generated += sent_count as u64;

        // Log throughput periodically
        if 0 == (state.total_generated & ((1u64<<13)-1)) {
            trace!("Generator: {} total messages sent", state.total_generated);
        }
    }

    info!("Generator shutting down. Total generated: {}", state.total_generated);
    Ok(())
}

/// Here we test the internal behavior of this actor
#[cfg(test)]
pub(crate) mod generator_tests {
    use std::thread::sleep;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_generator() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (generate_tx, generate_rx) = graph.channel_builder()
            .with_capacity(1024) // Larger capacity for testing
            .build();

        let state = new_state();
        graph.actor_builder()
            .with_name("UnitTest")
            .build(move |context| internal_behavior(context, generate_tx.clone(), state.clone()), SoloAct );

        graph.start();
        sleep(Duration::from_millis(100));
        graph.request_shutdown();

        graph.block_until_stopped(Duration::from_secs(1))?;

        // Should have many more messages due to batch processing
        let messages: Vec<u64> = generate_rx.testing_take_all();
        assert!(messages.len() >= 256); // At least one full batch
        assert_eq!(messages[0], 0);
        assert_eq!(messages[1], 1);
        Ok(())
    }
}
