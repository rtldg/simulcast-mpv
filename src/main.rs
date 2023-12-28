// SPDX-License-Identifier: AGPL-3.0-or-later
mod message;
mod client;
mod server;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None, flatten_help = true, disable_help_subcommand = true)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[command(flatten)]
	verbose: clap_verbosity_flag::Verbosity,
}

#[derive(Debug, Subcommand)]
enum Commands {
	Client {
		/// Relay-server used by both users for synchronization
		#[arg(
			long,
			env = "SIMULCAST_RELAY_URL",
			default_value = "https://simulcast.example.org/relay"
		)]
		relay_url: http::Uri,
	},
	Relay {
		/// Address to bind to for running a relay-server
		#[arg(long, default_value = "127.0.0.1")]
		bind_address: std::net::IpAddr,
		/// Port to bind to for running a relay-server
		#[arg(long, default_value_t = 30777)]
		bind_port: u16,
	},
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	dotenvy::dotenv()?; // load .env file if available for `SIMULCAST_RELAY_URL` variable
	let args = Cli::parse();

	match args.command {
		Commands::Relay {
			bind_address,
			bind_port,
		} => {

		}
		Commands::Client { relay_url } => {

		}
	}

	println!("Hello, world!");
	Ok(())
}
