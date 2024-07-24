// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2024 rtldg <rtldg@protonmail.com>

#![forbid(unsafe_code)]

mod client;
mod message;
mod server;

use clap::{Parser, Subcommand};
use log::info;
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
		/// Relay-server used by both users for synchronization.
		/// If this is empty then it'll read the server from https://github.com/rtldg/simulcast-mpv/blob/master/docs/servers.txt
		#[arg(long, env = "SIMULCAST_RELAY_URL")]
		relay_url: Option<http::Uri>,
		/// The room/code for both users to use for synchronizing.
		/// Rooms are based on the media-title/file-name so you could edit this for a little bit of "salt"
		#[arg(long, env = "SIMULCAST_RELAY_ROOM", default_value = "abcd1234")]
		relay_room: String,
		/// mpv's socket path (input-ipc-server) that we connect to.
		#[arg(long, env = "SIMULCAST_CLIENT_SOCK")]
		client_sock: String,
	},
	Relay {
		/// Address to bind to
		#[arg(long, env = "SIMULCAST_BIND_ADDRESS", default_value = "127.0.0.1")]
		bind_address: std::net::IpAddr,
		/// Port to bind to
		#[arg(long, env = "SIMULCAST_BIND_PORT", default_value_t = 30777)]
		bind_port: u16,
	},
	InputReader {
		/// mpv's socket path (input-ipc-server) that we connect to.
		#[arg(long, env = "SIMULCAST_CLIENT_SOCK")]
		client_sock: String,
	},
}

fn main() -> anyhow::Result<()> {
	// Hopefully load "mpv/scripts/simulcast-mpv.env".
	if let Ok(mut p) = std::env::current_exe() {
		p.set_file_name("simulcast-mpv.env");
		let _ = dotenvy::from_path(&p);
	}
	// Load "$PWD/simulcast-mpv.env" (which probably doesn't exist).
	let _ = dotenvy::from_filename_override("simulcast-mpv.env");

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
			Commands::InputReader { client_sock } => input_reader(client_sock),
		}
	} else {
		install()
	};
	info!("{:?}", res);
	res
}

fn input_reader(client_sock: String) -> anyhow::Result<()> {
	let mpv = mpvipc::Mpv::connect(&client_sock)?;
	println!("Please input a special room code (or nothing, to reset) then hit enter:");
	// std::io::stdout().flush().unwrap();
	let mut code = String::new();
	let _ = std::io::stdin().read_line(&mut code).unwrap();
	let _ = mpv.set_property("user-data/simulcast/input_reader", code.trim().to_string());
	Ok(())
}

fn install() -> anyhow::Result<()> {
	let current_exe = std::env::current_exe()?;

	let mut mpv_dir = None;

	if let Ok(var) = std::env::var("MPV_HOME") {
		mpv_dir = Some(var.into());
	}

	if cfg!(windows) {
		/*
		let parent = current_exe.parent().unwrap().to_owned();
		if parent.join("mpv.exe").exists() {
			let portable_config = current_exe.parent().unwrap().join("portable_config");
			if portable_config.exists() {
				scripts_dir = Some(portable_config);
			}
		}
		*/
		let portable_config = current_exe.parent().unwrap().join("portable_config");
		if portable_config.exists() {
			mpv_dir = Some(portable_config);
		}
	}

	let scripts_dir = mpv_dir
		.unwrap_or_else(|| {
			directories::UserDirs::new().unwrap().home_dir().join(if cfg!(windows) {
				"AppData\\Roaming\\mpv"
			} else {
				".config/mpv"
			})
		})
		.join("scripts");

	println!("- Creating {}", scripts_dir.display());
	std::fs::create_dir_all(&scripts_dir)?;

	// TODO: Option to not overwrite if the file exists...
	println!("- Writing  {}", scripts_dir.join("simulcast-mpv.lua").display());
	std::fs::write(scripts_dir.join("simulcast-mpv.lua"), include_str!("simulcast-mpv.lua"))?;

	let target_exe = scripts_dir.join(if cfg!(windows) {
		"simulcast-mpv.exe"
	} else {
		"simulcast-mpv"
	});
	if target_exe != current_exe {
		println!("- Copying current executable to scripts directory...");
		let _ = std::fs::copy(&current_exe, target_exe)?;
	}

	println!("Press ENTER to exit...");
	let _ = std::io::stdin().read(&mut [0u8]).unwrap();

	Ok(())
}
