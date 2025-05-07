mod reencrypt;
mod llm;
mod tee_attestation;
mod tee_mock_attestation;
mod test_commands;

use test_commands::{Cmd, CliArgument};
use clap::Parser;
use reencrypt::run_reencrypt_example;


#[tokio::main]
async fn main() -> color_eyre::Result<()> {

    color_eyre::install()?;
	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let cmd = Cmd::parse();

    match cmd.argument {
        CliArgument::TestUmbral => {
            run_reencrypt_example()?;
        }
        CliArgument::TestTee => {
            let (
                _tee_quote,
                tee_quote_bytes
            ) = tee_attestation::generate_tee_attestation_with_data([0; 64], false)?;
        }
    }

    Ok(())
}

