use steady_state::*;
use arg::MainArg;
mod arg;

pub(crate) mod actor {
    pub(crate) mod heartbeat;
    pub(crate) mod generator;
    pub(crate) mod worker;
    pub(crate) mod logger;
}

fn main() -> Result<(), Box<dyn Error>> {

    let cli_args = MainArg::parse();
    let _ = init_logging(LogLevel::Info);

    // Build high-performance graph with optimizations
    let mut graph = GraphBuilder::default()
        .build(cli_args);

    build_graph(&mut graph);

    //startup entire graph
    graph.start();
    // your graph is running here until actor calls graph stop
    graph.block_until_stopped(Duration::from_secs(5)) // Longer timeout for performance testing
}

const NAME_HEARTBEAT: &str = "heartbeat";
const NAME_GENERATOR: &str = "generator";
const NAME_WORKER: &str = "worker";
const NAME_LOGGER: &str = "logger";

fn build_graph(graph: &mut Graph) {
    let channel_builder = graph.channel_builder();

    // Use large channel capacities for high throughput
    let (heartbeat_tx, heartbeat_rx) = channel_builder
        .with_capacity(1024)  // Large buffer for heartbeat bursts
        .build();
    let (generator_tx, generator_rx) = channel_builder
        .with_capacity(8192)  // Very large buffer for high-speed generation
        .build();
    let (worker_tx, worker_rx) = channel_builder
        .with_capacity(8192)  // Large buffer for processed messages
        .build();

    let actor_builder = graph.actor_builder()
        .with_mcpu_avg();

    let mut team = ActorTeam::new(graph); //TODO: rewrite ...
        
    let state = new_state();
    actor_builder.with_name(NAME_HEARTBEAT)
        .build(move |context| {
            actor::heartbeat::run(context, heartbeat_tx.clone(), state.clone())
        }, &mut Threading::Join(&mut team));

    let state = new_state();
    actor_builder.with_name(NAME_GENERATOR)
        .build(move |context| {
            actor::generator::run(context, generator_tx.clone(), state.clone())
        }, &mut Threading::Join(&mut team));

    team.spawn();//this is used to lets multiple actors share the same "core" by putting them on the same team.
    
    let state = new_state();
    actor_builder.with_name(NAME_WORKER)
        .build(move |context| {
            actor::worker::run(context, heartbeat_rx.clone(), generator_rx.clone(), worker_tx.clone(), state.clone())
        }, &mut Threading::Spawn);

    let state = new_state();
    actor_builder.with_name(NAME_LOGGER)
        .build(move |context| {
            actor::logger::run(context, worker_rx.clone(), state.clone())
        }, &mut Threading::Spawn);
}

// TODO: show some core optimizations.

#[cfg(test)]
pub(crate) mod main_tests {
    use steady_state::*;
    use steady_state::graph_testing::{StageDirection, StageWaitFor};
    use crate::actor::worker::FizzBuzzMessage;
    use super::*;

    #[test]
    fn graph_test() -> Result<(), Box<dyn Error>> {

        let mut graph = GraphBuilder::for_testing().build(MainArg::default());

        build_graph(&mut graph);
        graph.start();

        let stage_manager = graph.stage_manager();
        // Test with larger numbers for performance validation
        stage_manager.actor_perform(NAME_GENERATOR, StageDirection::EchoAt(0, 1000u64))?;
        stage_manager.actor_perform(NAME_HEARTBEAT, StageDirection::Echo(100u64))?;
        stage_manager.actor_perform(NAME_LOGGER, StageWaitFor::Message(FizzBuzzMessage::FizzBuzz, Duration::from_secs(5)))?;
        stage_manager.final_bow();

        graph.request_stop();

        graph.block_until_stopped(Duration::from_secs(2))
    }
}
