use steady_state::*;
use arg::MainArg;
mod arg;

// The actor module contains all the actor implementations for this pipeline.
// Each actor is in its own submodule for clarity and separation of concerns.
pub(crate) mod actor {
    pub(crate) mod heartbeat;
    pub(crate) mod generator;
    pub(crate) mod worker_double_buffer;
    pub(crate) mod worker_zero_copy;
    pub(crate) mod logger;
}

fn main() -> Result<(), Box<dyn Error>> {
    // Parse command-line arguments (rate, beats, etc.) using clap.
    let cli_args = MainArg::parse();

    // Initialize logging at Info level for runtime diagnostics and performance output.
    init_logging(LogLevel::Info)?;

    // Build the actor graph with all channels and actors, using the parsed arguments.
    let mut graph = GraphBuilder::default()
        .with_telemtry_production_rate_ms(200) // slower telemetry frame rate, //##!##//
        .build(cli_args);

    // Construct the full actor pipeline and channel topology.
    build_graph(&mut graph);

    // Start the entire actor system. All actors and channels are now live.
    graph.start();

    // The system runs until an actor requests shutdown or the timeout is reached.
    // The timeout here is set longer to allow for high-throughput performance testing.
    graph.block_until_stopped(Duration::from_secs(20)) //let the large channels drain.
}

// Actor names for use in graph construction and testing.
const NAME_HEARTBEAT: &str = "heartbeat";
const NAME_GENERATOR: &str = "generator";
const NAME_WORKER: &str = "worker";
const NAME_LOGGER: &str = "logger";

fn build_graph(graph: &mut Graph) {
    // Channel builder is configured with advanced telemetry and alerting features.
    // - Red/orange alerts for congestion
    // - Percentile-based monitoring for channel fill levels
    // - Real-time average rate tracking
    let channel_builder = graph.channel_builder()
        // Smoother rates over a longer window
        .with_compute_refresh_window_floor(Duration::from_secs(4),Duration::from_secs(24))
        // Red alert if channel is >90% full on average (critical congestion)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        // Orange alert if channel is >60% full on average (early warning)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p60()), AlertColor::Orange)
        // Track average message rate for each channel
        .with_avg_rate();

    // Channel capacities are set extremely large for high-throughput, batch-friendly operation.
    // - Heartbeat channel: moderate size for timing signals
    // - Generator and worker channels: 1,048,576 messages (1<<20) for massive batch processing
    let (heartbeat_tx, heartbeat_rx) = channel_builder
        .with_capacity(1024)  // Large buffer for heartbeat bursts
        .build();
    let (generator_tx, generator_rx) = channel_builder // important for high volume throughput
        .with_capacity(1<<21)  // Large buffer for high-speed generation
        .build();
    let (worker_tx, worker_rx) = channel_builder // important for high volume throughput
        .with_capacity(1<<21)  // Large buffer for processed messages
        .build();

    // The actor builder is configured to collect thread/core info and load metrics.
    // - with_thread_info: enables reporting of OS thread and CPU core (requires core_affinity feature in Cargo.toml)
    // - with_load_avg, with_mcpu_avg: enables real-time load and CPU usage metrics
    let actor_builder = graph.actor_builder()
        .with_thread_info()
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m768()), AlertColor::Red)
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange)
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow)
        .with_load_avg()
        .with_mcpu_avg();

    // NOTE: The core_affinity and display features in Cargo.toml ensure that actors remain on their assigned CPU core.
    // This is critical for cache locality and consistent performance. Without core_affinity, actors could move between cores,
    // but would still not move between threads (each actor or team is always bound to a thread).

    // Actor grouping: Troupe (team) vs SoloAct
    // - MemberOf(&mut team): actors are grouped to share a single thread, cooperatively yielding to each other.
    //   This is optimal for lightweight actors or those that coordinate closely (e.g., generator and heartbeat).
    // - SoloAct: actor runs on its own dedicated thread, ideal for CPU-intensive or batch-heavy actors (e.g., worker, logger).
    let mut team = graph.actor_troupe();                                             //#!#//

    // Heartbeat actor: shares a thread with generator (MemberOf team)
    let state = new_state();
    actor_builder.with_name(NAME_HEARTBEAT)
        .build(move |context| 
            actor::heartbeat::run(context, heartbeat_tx.clone(), state.clone())
        ,  MemberOf(&mut team));   //#!#//

    // Generator actor: shares a thread with heartbeat (MemberOf team)
    let state = new_state();
    actor_builder.with_name(NAME_GENERATOR)
        .build(move |context|
            actor::generator::run(context, generator_tx.clone(), state.clone())
        ,  MemberOf(&mut team));   //#!#//
    drop(team); // this is when the troupe is finalized and started //#!#//

    // Worker actor: runs on its own thread (SoloAct) for maximum throughput and isolation
    let use_double_buffer = true;//#!#//

    if use_double_buffer {
        let state = new_state();
        actor_builder.with_name(NAME_WORKER)
            .build(move |context| 
                actor::worker_double_buffer::run(context, heartbeat_rx.clone(), generator_rx.clone(), worker_tx.clone(), state.clone())//#!#//
            , SoloAct);
    } else {
        let state = new_state();
        actor_builder.with_name(NAME_WORKER)
            .build(move |context| 
                actor::worker_zero_copy::run(context, heartbeat_rx.clone(), generator_rx.clone(), worker_tx.clone(), state.clone())//#!#//
            , SoloAct);
    }


    // Logger actor: runs on its own thread (SoloAct) for maximum throughput and isolation
    let state = new_state();
    actor_builder.with_name(NAME_LOGGER)
        .build(move |context|
            actor::logger::run(context, worker_rx.clone(), state.clone())
        , SoloAct);
}

#[cfg(test)]
pub(crate) mod main_tests {
    use steady_state::*;
    use steady_state::graph_testing::{StageDirection, StageWaitFor};
    use crate::actor::worker_double_buffer::FizzBuzzMessage;
    use super::*;

    // This test demonstrates orchestrated, multi-actor testing using the stage manager.
    // It allows precise control over actor behavior and verification of system interactions.
    #[test]
    fn graph_test() -> Result<(), Box<dyn Error>> {

        let mut graph = GraphBuilder::for_testing()
            .build(MainArg::default());

        build_graph(&mut graph);
        graph.start();

        // Stage management provides orchestrated testing of multi-actor scenarios.
        // This enables precise control over actor behavior and verification of
        // complex system interactions without manual coordination complexity.
        let stage_manager = graph.stage_manager();
        stage_manager.actor_perform(NAME_GENERATOR, StageDirection::Echo(15u64))?; // Sends a value to the generator
        stage_manager.actor_perform(NAME_HEARTBEAT, StageDirection::Echo(100u64))?; // Sends a value to the heartbeat
        stage_manager.actor_perform(NAME_LOGGER,    StageWaitFor::Message(FizzBuzzMessage::FizzBuzz
                                                                          , Duration::from_secs(2)))?; // Waits for a FizzBuzz message

        stage_manager.final_bow();
        graph.request_shutdown();

        graph.block_until_stopped(Duration::from_secs(5))
    }
}
