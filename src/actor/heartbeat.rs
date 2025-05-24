use steady_state::*;

/// High-performance heartbeat with batched operations
pub(crate) struct HeartbeatState {
    pub(crate) count: u64,
    pub(crate) beats_sent: u64,
    pub(crate) burst_size: usize, // Send heartbeats in bursts for efficiency
}

/// this is the normal entry point for our actor in the graph using its normal implementation
pub async fn run(context: SteadyContext, heartbeat_tx: SteadyTx<u64>, state: SteadyState<HeartbeatState>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor([], [&heartbeat_tx]);
    if cmd.use_internal_behavior {
        internal_behavior(cmd, heartbeat_tx, state).await
    } else {
        cmd.simulated_behavior(vec!(&heartbeat_tx)).await
    }
}

async fn internal_behavior<C: SteadyCommander>(mut cmd: C
                                               , heartbeat_tx: SteadyTx<u64>
                                               , state: SteadyState<HeartbeatState> ) -> Result<(),Box<dyn Error>> {
    let args = cmd.args::<crate::MainArg>().expect("unable to downcast");
    let rate = Duration::from_millis(args.rate_ms);
    let beats = args.beats;

    let mut state = state.lock(|| HeartbeatState{
        count: 0,
        beats_sent: 0,
        burst_size: 32, // Send heartbeats in bursts of 32
    }).await;

    let mut heartbeat_tx = heartbeat_tx.lock().await;

    // Pre-allocate burst buffer
    let mut burst = Vec::with_capacity(state.burst_size);

    //loop is_running until shutdown signal then we call the closure which closes our outgoing Tx
    while cmd.is_running(|| i!(heartbeat_tx.mark_closed())) {
        //await here until periodic time and room for burst
        await_for_all!(cmd.wait_periodic(rate),
                       cmd.wait_vacant(&mut heartbeat_tx, state.burst_size));

        // Prepare burst of heartbeat values
        burst.clear();
        for _ in 0..state.burst_size.min((beats - state.count) as usize) {
            if state.count >= beats {
                break;
            }
            burst.push(state.count);
            state.count += 1;
        }

        if !burst.is_empty() {
            // Send entire burst at once
            let sent_count = cmd.send_slice_until_full(&mut heartbeat_tx, &burst);
            state.beats_sent += sent_count as u64;

            // Adjust count if not all messages were sent
            if sent_count < burst.len() {
                state.count -= (burst.len() - sent_count) as u64;
            }

            trace!("Heartbeat burst: sent {} beats, total: {}", sent_count, state.beats_sent);
        }

        if state.count >= beats {
            info!("Heartbeat completed {} beats, requesting graph stop", beats);
            cmd.request_graph_stop().await;
        }
    }

    info!("Heartbeat shutting down. Total beats sent: {}", state.beats_sent);
    Ok(())
}

/// Here we test the internal behavior of this actor
#[cfg(test)]
pub(crate) mod heartbeat_tests {
    pub use std::thread::sleep;
    use steady_state::*;
    use crate::arg::MainArg;
    use super::*;

    #[test]
    fn test_heartbeat() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(MainArg {
            rate_ms: 10,
            beats: 100,
        });
        let (heartbeat_tx, heartbeat_rx) = graph.channel_builder()
            .with_capacity(512)
            .build();

        let state = new_state();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context|
                internal_behavior(context, heartbeat_tx.clone(), state.clone())
            );

        graph.start();
        sleep(Duration::from_millis(500));
        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(1))?;

        let beats: Vec<u64> = heartbeat_rx.testing_take_all();
        assert!(!beats.is_empty());
        assert_eq!(beats[0], 0);
        Ok(())
    }
}
