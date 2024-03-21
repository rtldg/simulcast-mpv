// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2024 rtldg <rtldg@protonmail.com>

#![forbid(unsafe_code)]

mod client;
mod message;
mod server;

use clap::{Parser, Subcommand};
use log::info;
#[allow(unused_imports)] // for when I'm testing and have the "Press any key" disabled
use std::io::Read;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None, flatten_help = true, disable_help_subcommand = true, infer_subcommands = true)]
struct Cli {
	#[command(subcommand)]
	command: Option<Commands>,
	/// -q silences output
	/// -v show warnings
	/// -vv show info
	/// -vvv show debug
	/// -vvvv show trace
	#[command(flatten)]
	verbose: clap_verbosity_flag::Verbosity<clap_verbosity_flag::InfoLevel>,
}

#[derive(Debug, Subcommand)]
enum Commands {
	Client {
		/// Relay-server used by both users for synchronization
		#[arg(
			long,
			env = "SIMULCAST_RELAY_URL",
			default_value = "wss://simulcast-mpv.fly.dev/"
		)]
		relay_url: http::Uri,
		/// The room/code for both users to use for synchronizing.
		/// Rooms are based on the media-title/file-name so you could edit this for a little bit of "salt"
		#[arg(long, env = "SIMULCAST_RELAY_ROOM", default_value = "abc123")]
		relay_room: String,
		/// mpv's socket path (input-ipc-server) that we connect to.
		#[arg(long, env = "SIMULCAST_CLIENT_SOCK")]
		client_sock: String,
	},
	Relay {
		/// Address to bind to
		#[arg(long, default_value = "127.0.0.1")]
		bind_address: std::net::IpAddr,
		/// Port to bind to
		#[arg(long, default_value_t = 30777)]
		bind_port: u16,
	},
}

fn main() -> anyhow::Result<()> {
	// Hopefully load "mpv/scripts/.env".
	if let Ok(mut p) = std::env::current_exe() {
		p.set_file_name(".env");
		let _ = dotenvy::from_path(&p);
	}
	// Load "$PWD/.env" (which probably doesn't exist).
	let _ = dotenvy::dotenv();

	let args = Cli::parse();

	let res = if let Some(command) = args.command {
		match command {
			Commands::Relay {
				bind_address,
				bind_port,
			} => server::server(args.verbose.log_level_filter(), bind_address, bind_port),
			Commands::Client {
				relay_url,
				relay_room,
				client_sock,
			} => client::client(args.verbose.log_level_filter(), relay_url, relay_room, client_sock),
		}
	} else {
		install()
	};
	info!("{:?}", res);
	res
}

fn install() -> anyhow::Result<()> {
	let scripts_dir = directories::UserDirs::new().unwrap().home_dir().join(if cfg!(windows) {
		"AppData\\Roaming\\mpv\\scripts\\"
	} else {
		".config/mpv/scripts/"
	});

	println!("- Creating {}", scripts_dir.display());
	std::fs::create_dir_all(&scripts_dir)?;

	// TODO: Option to not overwrite if the file exists...
	println!("- Writing {}", scripts_dir.join("simulcast-mpv.lua").display());
	std::fs::write(scripts_dir.join("simulcast-mpv.lua"), include_str!("simulcast-mpv.lua"))?;

	let target_exe = scripts_dir.join(if cfg!(windows) {
		"simulcast-mpv.exe"
	} else {
		"simulcast-mpv"
	});
	if target_exe != std::env::current_exe()? {
		println!("- Copying current executable to scripts directory...");
		let _ = std::fs::copy(std::env::current_exe()?, target_exe)?;
	}

	println!("Press any key to continue...");
	let _ = std::io::stdin().read(&mut [0u8]).unwrap();

	Ok(())
}
