// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2025 rtldg <rtldg@protonmail.com>

#![forbid(unsafe_code)]

#[cfg(feature = "client")]
mod client;
mod message;
#[cfg(feature = "client")]
mod mpvipc;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
use anyhow::Context;
use clap::{Parser, Subcommand};
use log::info;
#[cfg(feature = "client")]
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
	/// Toggles whether to block and wait for you to press ENTER during install.
	#[cfg(feature = "client")]
	#[arg(long, default_value_t = false)]
	noninteractive: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
	#[cfg(feature = "client")]
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
	#[cfg(feature = "server")]
	Relay {
		/// Address to bind to
		#[arg(long, env = "SIMULCAST_BIND_ADDRESS", default_value = "127.0.0.1")]
		bind_address: std::net::IpAddr,
		/// Port to bind to
		#[arg(long, env = "SIMULCAST_BIND_PORT", default_value_t = 30777)]
		bind_port: u16,
		/// Repository URL (for AGPL-3.0 reasons).
		#[arg(long, env = "SIMULCAST_REPO_URL")]
		repo_url: http::Uri,
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

	if let Some(command) = args.command {
		let res = match command {
			#[cfg(feature = "server")]
			Commands::Relay {
				bind_address,
				bind_port,
				repo_url,
			} => server::server(args.verbose.log_level_filter(), bind_address, bind_port, &repo_url),
			#[cfg(feature = "client")]
			Commands::Client {
				relay_url,
				relay_room,
				client_sock,
			} => client::client(args.verbose.log_level_filter(), relay_url, relay_room, client_sock),
		};
		info!("res = {res:?}");
		res
	} else {
		#[cfg(feature = "client")]
		{
			let res = install();
			if args.noninteractive {
				res
			} else {
				if let Err(e) = res {
					println!("\n{e:?}");
				}
				println!("\nPress ENTER to exit...");
				let _ = std::io::stdin().read(&mut [0u8]).unwrap();
				Ok(()) // Slurp it so it doesn't double print...
			}
		}
		#[cfg(not(feature = "client"))]
		{
			Ok(())
		}
	}
}

#[cfg(feature = "client")]
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
	std::fs::create_dir_all(&scripts_dir).with_context(|| format!("Failed to create {}", scripts_dir.display()))?;

	// TODO: Option to not overwrite if the file exists...
	let lua_file = scripts_dir.join("simulcast-mpv.lua");
	println!("- Writing  {}", lua_file.display());
	std::fs::write(&lua_file, include_str!("simulcast-mpv.lua"))
		.with_context(|| format!("Failed to write {}", lua_file.display()))?;

	let target_exe = scripts_dir.join(if cfg!(windows) {
		"simulcast-mpv.exe"
	} else {
		"simulcast-mpv"
	});
	if target_exe != current_exe {
		println!("- Writing  {}", target_exe.display());
		let mut tmp_exe = target_exe.clone();
		tmp_exe.set_extension(".tmp");
		let _ =
			std::fs::copy(&current_exe, &tmp_exe).with_context(|| format!("Failed to write {}", tmp_exe.display()))?;
		let _ = std::fs::rename(&tmp_exe, &target_exe)
			.with_context(|| format!("Failed to rename {} to {}", tmp_exe.display(), target_exe.display()))?;
	}

	println!("\nDONE!");

	Ok(())
}
