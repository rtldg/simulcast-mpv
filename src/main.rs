// SPDX-License-Identifier: AGPL-3.0-or-later

mod client;
mod install;
mod message;
mod server;

use clap::{Parser, Subcommand};
use clap_verbosity_flag::Verbosity;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None, flatten_help = true, disable_help_subcommand = true, infer_subcommands = true)]
struct Cli {
	#[command(subcommand)]
	command: Option<Commands>,
	#[command(flatten)]
	verbose: Verbosity,
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
		/// The room/code for both users to use for synchronizing.
		/// There's not much for access-control so this is a basic solution.
		#[arg(long, env = "SIMULCAST_RELAY_ROOM", default_value = "abc123")]
		relay_room: String,
		/// Socket path used by mpv that we will connect to.
		#[arg(long, env = "SIMULCAST_CLIENT_SOCK")]
		client_sock: String,
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

fn main() -> anyhow::Result<()> {
	let _ = dotenvy::dotenv(); // load .env if available
	let args = Cli::parse();

	if let Some(command) = args.command {
		match command {
			Commands::Relay {
				bind_address,
				bind_port,
			} => server::server(args.verbose, bind_address, bind_port),
			Commands::Client {
				relay_url,
				relay_room,
				client_sock,
			} => client::client(args.verbose, relay_url, relay_room, client_sock),
		}
	} else {
		install::install()
	}
}
