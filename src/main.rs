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
    let channel_builder = graph.channel_builder()
        // Threshold-based alerting enables proactive monitoring of system health.
        // Red alerts indicate critical congestion requiring immediate attention,
        // while orange alerts provide early warning of developing bottlenecks.
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p60()), AlertColor::Orange)
        // Percentile monitoring provides statistical insight into channel utilization.
        // The 80th percentile balances responsiveness to load spikes with stability
        // against transient fluctuations in message flow rates.
        .with_avg_rate()
        .with_filled_percentile(Percentile::p80());
        //message rate at the 80th percentile
        //.with_rate_percentile(Percentile::p80()); //TODO fix::
    
    
    // Use large channel capacities for high throughput
    let (heartbeat_tx, heartbeat_rx) = channel_builder
        .with_capacity(1024)  // Large buffer for heartbeat bursts
        .build();
    let (generator_tx, generator_rx) = channel_builder
        .with_capacity(65536*4)  // Very large buffer for high-speed generation
        .build();
    let (worker_tx, worker_rx) = channel_builder
        .with_capacity(65536*2)  // Large buffer for processed messages
        .build();

    let actor_builder = graph.actor_builder()
        .with_thread_info()
        .with_load_avg()
        .with_mcpu_avg();

    //NOTE the Cargo.toml has set the features for core_affinity and display.  This ensures the
    // actors remain on the core where they are placed to allow for optimal core cache usage.
    
    //Note that with this pattern, we can join actors together under the same thread so they
    //end up cooperatively sharing upon await. This can be optimal when combining actors with
    //low compute demand or when combining actors in series where one waits on the next.
    let mut team = graph.actor_team();
    
    let state = new_state();
    actor_builder.with_name(NAME_HEARTBEAT)
        .build(move |context| {
            actor::heartbeat::run(context, heartbeat_tx.clone(), state.clone())
        },  &mut Threading::Join(&mut team));

    let state = new_state();
    actor_builder.with_name(NAME_GENERATOR)
        .build(move |context| {
            actor::generator::run(context, generator_tx.clone(), state.clone())
        },  &mut Threading::Join(&mut team));

    //this is used to lets multiple actors share the same "thread" by putting them on the same team.
    //once we are done adding actors we must call spawn();
    team.spawn();
    

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


#[cfg(test)]
pub(crate) mod main_tests {
    use steady_state::*;
    use steady_state::graph_testing::{StageDirection, StageWaitFor};
    use crate::actor::worker::FizzBuzzMessage;
    use super::*;

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
     
        //   stage_manager.actor_perform(NAME_GENERATOR, StageDirection::Echo(15u64))?;
       // stage_manager.actor_perform(NAME_HEARTBEAT, StageDirection::Echo(100u64))?;
       // stage_manager.actor_perform(NAME_LOGGER,    StageWaitFor::Message(FizzBuzzMessage::FizzBuzz
          //                                                                , Duration::from_secs(2)))?;
        
         stage_manager.final_bow();
        // 
         graph.request_shutdown();
         
         graph.block_until_stopped(Duration::from_secs(5))
      // Ok(())
    }
}
