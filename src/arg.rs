use clap::Parser;

/// Command-line arguments for the Steady State application
#[derive(Parser, Debug, PartialEq, Clone)]
pub(crate) struct MainArg {
    /// Rate in milliseconds between actor operations (e.g., heartbeats)
    #[arg(short = 'r', long = "rate", default_value = "10")]
    pub(crate) rate_ms: u64,

    /// Number of beats (loop iterations before shutdown)
    #[arg(short = 'b', long = "beats", default_value = "12000")]
    pub(crate) beats: u64,
}

impl Default for MainArg {
    fn default() -> Self {
        MainArg {
            rate_ms: 100,
            beats: 600,
        }
    }
}
