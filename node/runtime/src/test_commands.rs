use clap::Parser;
use serde::{Deserialize, Serialize};


#[derive(Parser, Serialize, Deserialize, Clone, Debug)]
#[clap(name = "runtime")]
pub(crate) struct Cmd {
    #[clap(subcommand)]
    pub argument: CliArgument,
}

#[derive(Debug, Parser, Clone, Deserialize, Serialize)]
pub enum CliArgument {
    TestUmbral,
    TestTee,
}
