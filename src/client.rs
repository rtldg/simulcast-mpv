// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2024 rtldg <rtldg@protonmail.com>

// https://github.com/snapview/tungstenite-rs

// https://crates.io/crates/shadow-rs
//    A build-time information stored in your rust project

// https://mpv.io/manual/master/
// https://github.dev/mpv-player/mpv
// https://docs.rs/mpvipc/

// cargo run --release -- client --client-sock mpvsock42

// cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17
// cargo +1.75 build --release
// rustup override set 1.75 # last version before win7 support died

use anyhow::Context;
use log::debug;
use log::error;
use log::info;
use mpvipc::MpvDataType;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;

use anyhow::anyhow;
use mpvipc::{Event, Mpv, MpvCommand, Property};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;

use crate::message::WsMessage;

struct SharedState {
	party_count: u32,
	paused: bool,
	time: f64,
	room_code: String,
	room_hash: String,
}

fn get_room_hash(mut code: String, relay_room: &str) -> String {
	code.push_str(relay_room);
	blake3::hash(code.as_bytes()).to_hex().to_string()
}

async fn ws_thread(
	relay_url: String,
	mpv: &Mpv,
	receiver: &mut UnboundedReceiver<WsMessage>,
	state: Arc<Mutex<SharedState>>,
) -> anyhow::Result<()> {
	info!("ws_thread!");

	loop {
		// nom nom nom. eat messages.
		match receiver.try_recv() {
			Ok(_) => (),
			Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
			Err(e) => Err(e)?,
		}
	}

	let (mut ws, _) = tokio_tungstenite::connect_async(relay_url)
		.await
		.context("Failed to setup websocket connection")?;

	info!("connected to websocket");

	{
		let room_hash = {
			let state = state.lock().unwrap();
			state.room_hash.clone()
		};
		let joinmsg = WsMessage::Join(room_hash);
		debug!("send msg = {joinmsg:?}");
		ws.send(Message::Text(serde_json::to_string(&joinmsg).unwrap())).await?;
	}

	loop {
		tokio::select! {
			msg = receiver.recv() => {
				let msg = msg.unwrap();
				match msg {
					WsMessage::Ping(_) | WsMessage::Pong(_) => (),
					_ => debug!("send msg = {msg:?}")
				}
				ws.send(
					Message::Text(
						serde_json::to_string(&msg).unwrap()
					)
				).await?;
			}
			msg = ws.next() => {
				let msg: WsMessage = serde_json::from_str(&msg.unwrap()?.into_text()?)?;
				match msg {
					WsMessage::Ping(_) | WsMessage::Pong(_) => (),
					_ => debug!("recv msg = {msg:?}")
				}
				match msg {
					WsMessage::Join(_) => { /* we shouldn't be receiving this */ },
					WsMessage::Party(count) => {
						let should_pause = {
							let mut state = state.lock().unwrap();
							if state.party_count < 2 && count == 1 {
								// user is solo-watching and probably just opened mpv...
							} else {
								state.paused = true;
							}
							state.party_count = count;
							state.paused
						};

						if should_pause {
							// these can hit too early and cause `Err(MpvError: property unavailable)`?
							let _ = mpv.set_property("pause", true);
							let _ = mpv.set_property("speed", 1.0); // useful for me

							let _ = mpv.run_command(MpvCommand::ShowText {
								text: format!("party count: {count}"),
								duration_ms: Some(2000),
								level: None,
							});
						}
					},
					WsMessage::Resume => {
						{
							let mut state = state.lock().unwrap();
							state.paused = false;
						}
						mpv.set_property("pause", false)?;
					},
					WsMessage::AbsoluteSeek(time) => {
						{
							let mut state = state.lock().unwrap();
							state.paused = true;
							state.time = time;
						}
						mpv.set_property("pause", true)?;
						// mpv.seek(time, mpvipc::SeekOptions::Absolute)?; // Not exact seek?
						// mpv.set_property("playback-time", time)?; // Doesn't show OSD
						mpv.run_command_raw2(&["osd-auto", "seek"], &[
							&time.to_string(),
							"absolute+exact"
						])?;
					},
					WsMessage::Ping(s) => {
						ws.send(Message::text(serde_json::to_string(&WsMessage::Pong(s)).unwrap())).await?;
					},
					WsMessage::Pong(_) => { /* we shouldn't be reciving this */},
				}
			}
		}
	}
}

fn spawn_input_reader(client_sock: String) -> anyhow::Result<()> {
	let exe = std::env::current_exe()?;
	#[cfg(windows)]
	{
		const CREATE_NEW_CONSOLE: u32 = 0x00000010;
		let _ = std::process::Command::new(exe)
			.args(&["input-reader", "--client-sock", &client_sock])
			.creation_flags(CREATE_NEW_CONSOLE)
			.spawn()?
			.wait()?;
	}
	#[cfg(not(windows))]
	{
		// TODO... Spawn xterm maybe...
		let _ = exe;
		let _ = client_sock;
	}
	Ok(())
}

pub fn client(
	verbosity: log::LevelFilter,
	relay_url: Option<http::Uri>,
	relay_room: String,
	client_sock: String,
) -> anyhow::Result<()> {
	let verbosity = if true { log::LevelFilter::Debug } else { verbosity };
	flexi_logger::Logger::with(
		flexi_logger::LogSpecification::builder()
			.default(verbosity)
			.module("mpvipc", log::LevelFilter::Error)
			.module("rustls", log::LevelFilter::Warn)
			.module("tungstenite", log::LevelFilter::Warn)
			.build(),
	)
	.format(flexi_logger::detailed_format)
	.log_to_stdout()
	.log_to_file(flexi_logger::FileSpec::default().directory(std::env::temp_dir()))
	// .log_to_file(flexi_logger::FileSpec::try_from("simulcast.log")?)
	.start()?;
	// simple_logging::log_to_file("out.log", verbosity)?;

	// TODO: include git revision...?
	info!("simulcast-mpv version {}!", env!("CARGO_PKG_VERSION"));

	let relay_url = if relay_url.is_none() {
		// TODO: check list of urls to see if they're alive?
		info!("querying server from https://rtldg.github.io/simulcast-mpv/servers.txt ...");
		// github.io url used because it's cdn-backed and probably won't bother github too much if we fetch it all the time
		reqwest::blocking::Client::new()
			.get("https://rtldg.github.io/simulcast-mpv/servers.txt")
			.header(
				"user-agent",
				format!(
					"{}/{} ({})",
					env!("CARGO_PKG_NAME"),
					env!("CARGO_PKG_VERSION"),
					env!("CARGO_PKG_REPOSITORY")
				),
			)
			.send()?
			.text()?
			.lines()
			.next()
			.unwrap()
			.trim()
			.parse()?
	} else {
		relay_url.unwrap()
	};

	// TODO: Throw error messages up on mpv's screen too...
	if relay_url.host().is_none() {
		return Err(anyhow!("relay url is missing a host. url: '{relay_url}'"));
	}
	if relay_url.scheme_str() != Some("ws") && relay_url.scheme_str() != Some("wss") {
		return Err(anyhow!(
			"relay url scheme must be 'ws://' or 'wss://'. url: '{relay_url}'"
		));
	}

	info!("relay_url = '{relay_url}'");

	// I was originally testing with hardcoded socket names but a typo that put me back an hour...
	// Commands, get/set_property will eat events in the queue so let's separate the sockets...
	let mut mpv_events =
		Mpv::connect(&client_sock).context(format!("failed to connect to mpv socket '{}'", client_sock))?;
	let mpv_query = Mpv::connect(&client_sock)?;
	//
	let mpv_ws = Mpv::connect(&client_sock)?;

	info!("mpv objects setup...");

	let heartbeat_sock = client_sock.clone();
	let _ = std::thread::spawn(move || {
		let mpv_heartbeat = Mpv::connect(&heartbeat_sock).unwrap();
		for i in 1..usize::MAX {
			std::thread::sleep(Duration::from_secs_f64(0.1));
			mpv_heartbeat.set_property("user-data/simulcast/heartbeat", i).unwrap();
		}
	});

	let file = if let Ok(filename) = mpv_query.get_property_string("filename") {
		info!("file = '{filename}'");
		filename
	} else {
		rand::random::<u64>().to_string()
	};

	let state = Arc::new(Mutex::new(SharedState {
		party_count: 0,
		paused: false,
		time: 0.0,
		room_code: String::new(),
		room_hash: get_room_hash(file, &relay_room),
	}));

	let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();

	let rt = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.worker_threads(2)
		.build()?;
	let state_ws = state.clone();
	rt.spawn(async move {
		loop {
			let err = ws_thread(relay_url.to_string(), &mpv_ws, &mut receiver, state_ws.clone()).await;
			if let Err(err) = err {
				error!("{:?}", err);
			}
			{
				let mut state = state_ws.lock().unwrap();
				state.party_count = 0;
			}
		}
	});

	mpv_events.observe_property(1, "filename")?;
	mpv_events.observe_property(2, "pause")?;
	mpv_events.observe_property(3, "playback-time")?;
	mpv_events.observe_property(4, "user-data/simulcast/fuckmpv")?;
	mpv_events.observe_property(5, "user-data/simulcast/input_reader")?;

	// let mut tick = 0;
	#[allow(non_snake_case)]
	let (mut A_spam_last, mut A_spam_count, mut A_spam_cooldown) =
		(std::time::SystemTime::now(), 0, std::time::SystemTime::UNIX_EPOCH);

	loop {
		match mpv_events.event_listen()? {
			Event::Shutdown => return Ok(()),
			Event::Unimplemented => {}
			Event::PropertyChange { id: _, property } => match property {
				Property::Pause(paused) => {
					info!("pause called");

					let Ok(time) = mpv_query.get_property("playback-time/full") else {
						continue;
					};
					let mut state = state.lock().unwrap();

					if paused == state.paused {
						continue;
					}

					state.time = time;

					if state.party_count < 2 {
						state.paused = paused;
						continue;
					}

					info!("about to do pause stuff. state={}, new={}", state.paused, paused);

					state.paused = true;
					drop(state);

					if paused {
						let _ = sender.send(WsMessage::AbsoluteSeek(time));
					} else {
						// if we are here then we probably unpaused with the onscreen-display
						mpv_query.set_property("pause", true)?;
						let _ = sender.send(WsMessage::Resume);
					}
				}
				Property::Unknown { name, data } => match name.as_str() {
					"filename" => {
						let room_hash = {
							let mut state = state.lock().unwrap();
							if !state.room_code.is_empty() {
								// The roomid should:tm: still be valid.
								continue;
							} else {
								state.party_count = 0;
								match data {
									MpvDataType::String(s) => {
										state.room_hash = get_room_hash(s, &relay_room);
									}
									_ => (),
								}
							}
							state.room_hash.clone()
						};
						let _ = sender.send(WsMessage::Join(room_hash));
					}
					"user-data/simulcast/fuckmpv" => {
						let MpvDataType::String(data) = data else {
							// tf?
							continue;
						};

						if data == "." {
							continue;
						}

						info!("user-data/simulcast/fuckmpv = '{data}'");
						mpv_query.set_property("user-data/simulcast/fuckmpv", ".".to_string())?;

						if data == "queue_resume" {
							if state.lock().unwrap().party_count < 2 {
								mpv_query.set_property("pause", false)?;
								continue;
							}

							// let time: f64 = mpv_query.get_property("playback-time/full")?;
							// sender.send(WsMessage::AbsoluteSeek(time))?;
							let _ = sender.send(WsMessage::Resume);
						} else if data == "print_info" {
							if A_spam_last.elapsed()? > Duration::from_secs(2) {
								A_spam_count = 0;
								A_spam_cooldown = std::time::SystemTime::UNIX_EPOCH;
							}

							A_spam_count += 1;
							A_spam_last = std::time::SystemTime::now();

							if A_spam_count > 3 && A_spam_cooldown.elapsed()? > Duration::from_secs(2) {
								A_spam_cooldown = std::time::SystemTime::now();
								let input_reader_sock = client_sock.clone();
								let _ = std::thread::spawn(|| spawn_input_reader(input_reader_sock));
								// do prompt for custom room code...
							}

							// holy shit I hate Lua
							let (party_count, room_code, room_hash) = {
								let state = state.lock().unwrap();
								(state.party_count, state.room_code.clone(), state.room_hash.clone())
							};
							let _ = mpv_query.run_command(MpvCommand::ShowText {
								text: format!("SIMULCAST\nparty count = {party_count}\ncustom room code = '{room_code}'\nroom id/hash = {room_hash}"),
								duration_ms: Some(7000),
								level: None,
							});
						}
					}
					"user-data/simulcast/input_reader" => {
						let MpvDataType::String(data) = data else {
							// tf?
							continue;
						};

						let room_hash = {
							let mut state = state.lock().unwrap();
							state.room_code = data;
							if !state.room_code.is_empty() {
								state.room_hash = get_room_hash(state.room_code.clone(), &relay_room);
							} else {
								state.room_hash = get_room_hash(
									mpv_query
										.get_property_string("filename")
										.unwrap_or_else(|_| rand::random::<u64>().to_string()),
									&relay_room,
								);
							}
							state.room_hash.clone()
						};
						let _ = sender.send(WsMessage::Join(room_hash));
					}
					_ => {}
				},
				Property::PlaybackTime(_t) => {
					// tick += 1;
					// mpv.run_command(MpvCommand::ShowText {
					// 	text: tick.to_string(),
					// 	duration_ms: Some(100),
					// 	level: None,
					// })?;
				}
				_ => {}
			},
			Event::Seek => {
				// This is dumb but necessary. We need *some* wait here otherwise it's desynced.
				// Related place to edit in server.rs. Ctrl+f "BROCCOLI".
				std::thread::sleep(Duration::from_millis(100));

				let time: f64 = mpv_query.get_property("playback-time/full")?;
				let paused: bool = mpv_query.get_property("pause")?;
				let mut state = state.lock().unwrap();

				info!("Event::Seek. time = {}. expected = {}", time, state.time);

				if (time - state.time).abs() > 0.03 {
					// seems like we seeked...

					state.time = time;
					let party_count = state.party_count;

					if party_count > 1 {
						state.paused = true;
					}

					drop(state);

					if party_count > 1 && !paused {
						mpv_query.set_property("pause", true)?;
					}

					if party_count > 1 {
						let _ = sender.send(WsMessage::AbsoluteSeek(time));
					}
				}
			}
			_ => {}
		}
	}
}
