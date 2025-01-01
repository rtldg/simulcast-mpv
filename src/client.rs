// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2025 rtldg <rtldg@protonmail.com>

// https://crates.io/crates/shadow-rs
//    A build-time information stored in your rust project

// https://mpv.io/manual/master/
// https://github.dev/mpv-player/mpv

use anyhow::Context;
use log::debug;
use log::error;
use log::info;
use serde_json::json;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

use futures::SinkExt;
use futures::StreamExt;

use crate::mpvipc::Mpv;
use anyhow::anyhow;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::message::WsMessage;

struct SharedState {
	party_count: u32,
	paused: bool,
	time: f64,
	room_code: String,
	room_hash: String,
}

fn get_room_hash(code: &str, relay_room: &str) -> String {
	let code = code
		.chars()
		.map(|c| match c {
			'_' | '-' | '+' | '.' => ' ',
			_ => c,
		})
		.collect::<String>()
		+ relay_room;
	blake3::hash(code.as_bytes()).to_hex().to_string()
}

async fn ws_thread(
	relay_url: String,
	mpv: &mut Mpv,
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

	ws.send(WsMessage::Info(String::new()).to_websocket_msg()).await?;

	{
		let room_hash = {
			let state = state.lock().unwrap();
			state.room_hash.clone()
		};
		ws.send(WsMessage::Join(room_hash).send_helper()).await?;
	}

	// Using an `Instant` instead of `intervals_since_last_ping` because it's less prone to breaking in case the interval duration is ever changed for some reason.
	let mut last_ping_time = std::time::Instant::now();

	let mut interval = tokio::time::interval(Duration::from_secs(1));
	loop {
		tokio::select! {
			_ = interval.tick() => {
				if last_ping_time.elapsed() > Duration::from_secs(10) {
					anyhow::bail!("server hasn't pinged for 10s and we probably lost connection."); // anyhow::bail!() will return btw...
				}
			}
			msg = receiver.recv() => {
				let Some(msg) = msg else {
					// Sender has closed and the program is about to exit....
					let _ = ws.close(
						Some(CloseFrame {
							code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
							reason: "".into(),
						})
					).await; // could be canceled if the Runtime is dropped fast
					return Ok(());
				};
				ws.send(msg.send_helper()).await?;
			}
			msg = ws.next() => {
				let msg = msg.unwrap()?.into_text()?;
				let Ok(msg) = serde_json::from_str(&msg) else {
					debug!("unknown message = '{msg}'");
					continue;
				};
				match msg {
					WsMessage::Ping(_) | WsMessage::Pong(_) => (),
					_ => debug!("recv msg = {msg:?}")
				}
				match msg {
					WsMessage::Info(s) => {
						info!("server info: {s}");
					},
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
							let _ = mpv.set_property("pause", &json!(true));
							let _ = mpv.set_property("speed", &json!(1.0)); // useful for me (since I have my default mpv speed at 1.5x)

							let _ = mpv.show_text(&format!("party count: {count}"), Some(2000), None);
						}
					},
					WsMessage::Resume => {
						{
							let mut state = state.lock().unwrap();
							state.paused = false;
						}
						mpv.set_property("pause", &json!(false))?;
					},
					WsMessage::AbsoluteSeek(time) => {
						{
							let mut state = state.lock().unwrap();
							state.paused = true;
							state.time = time;
						}
						mpv.set_property("pause", &json!(true))?;
						// "osd-auto" is a prefix to make it show the onscreen-display seek bar just like seek binds do
						let _ = mpv.raw_command(&json!(["osd-auto", "seek", time.to_string(), "absolute+exact"]))?;
					},
					WsMessage::Ping(s) => {
						last_ping_time = std::time::Instant::now();
						ws.send(WsMessage::Pong(s).to_websocket_msg()).await?;
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
			.args(["input-reader", "--client-sock", &client_sock])
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
	let rt = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.worker_threads(2)
		.build()?;
	let res = client_inner(verbosity, relay_url, relay_room, client_sock, &rt);
	// mainly wait for our websocket connection to close...
	rt.shutdown_timeout(Duration::from_secs_f64(0.5));
	res
}

fn client_inner(
	verbosity: log::LevelFilter,
	relay_url: Option<http::Uri>,
	relay_room: String,
	client_sock: String,
	rt: &Runtime,
) -> anyhow::Result<()> {
	let verbosity = if true { log::LevelFilter::Debug } else { verbosity };
	flexi_logger::Logger::with(
		flexi_logger::LogSpecification::builder()
			.default(verbosity)
			.module("rustls", log::LevelFilter::Warn)
			.module("tokio_tungstenite", log::LevelFilter::Warn)
			.module("tungstenite", log::LevelFilter::Warn)
			.build(),
	)
	.format(flexi_logger::detailed_format)
	.log_to_stdout()
	.log_to_file(flexi_logger::FileSpec::default().directory(std::env::temp_dir()))
	// .log_to_file(flexi_logger::FileSpec::try_from("simulcast.log")?)
	.start()?;
	// simple_logging::log_to_file("out.log", verbosity)?;

	log_panics::init();

	// TODO: include git revision...?
	info!("simulcast-mpv version {}!", env!("CARGO_PKG_VERSION"));

	let relay_url = if let Some(relay_url) = relay_url {
		relay_url
	} else {
		// TODO: check list of urls to see if they're alive?
		info!("querying server from https://rtldg.github.io/simulcast-mpv/servers.txt ...");
		// github.io url used because it's cdn-backed and probably won't bother github too much if we fetch it all the time
		let resp = rt.block_on(async {
			reqwest::Client::new()
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
				.send()
				.await
		})?;
		rt.block_on(async { resp.text().await })?
			.lines()
			.next()
			.unwrap()
			.trim()
			.parse()?
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

	// The previously-used mpvipc crate would potentially eat events, which isn't optimal.
	// It's still easier to separate sockets for events & querying to help minimize
	// the chance of bugs until I finish more TODOs in mpvipc.rs
	let mut mpv_events =
		Mpv::connect(&client_sock).context(format!("failed to connect to mpv socket '{}'", client_sock))?;
	let mut mpv_query = Mpv::connect(&client_sock)?;
	mpv_query.events(false);
	let mut mpv_ws = Mpv::connect(&client_sock)?;
	mpv_ws.events(false);

	info!("mpv objects are setup...");

	let heartbeat_sock = client_sock.clone();
	let _ = std::thread::spawn(move || {
		let mut mpv_heartbeat = Mpv::connect(&heartbeat_sock).unwrap();
		mpv_heartbeat.events(false);
		// with a 32-bit build: it'd take 13.6y to finish this loop 😇
		for i in 1..usize::MAX {
			std::thread::sleep(Duration::from_secs_f64(0.1));
			if let Err(_) = mpv_heartbeat.set_property("user-data/simulcast/heartbeat", &json!(i)) {
				// mpv most likely exited (or if the property setting is failing: everything is already fucked!)
				return;
			}
		}
	});

	let file = if let Ok(filename) = mpv_query.get_property("filename") {
		let filename = filename.as_str().unwrap();
		info!("file = '{filename}'");
		filename.to_string()
	} else {
		rand::random::<u64>().to_string()
	};

	let state = Arc::new(Mutex::new(SharedState {
		party_count: 0,
		paused: false,
		time: 0.0,
		room_code: String::new(),
		room_hash: get_room_hash(&file, &relay_room),
	}));

	let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
	let state_ws = state.clone();
	rt.spawn(async move {
		loop {
			let err = ws_thread(relay_url.to_string(), &mut mpv_ws, &mut receiver, state_ws.clone()).await;
			if let Err(err) = err {
				error!("{:?}", err);
			} else {
				// Sender/receiver closed and ws_thread returned because the program is about to exit.
				return;
			}
			{
				let mut state = state_ws.lock().unwrap();
				state.party_count = 0;
			}
			tokio::time::sleep(Duration::from_secs_f64(3.1415926535897932384626433)).await;
		}
	});

	mpv_events.observe_property(1, "filename")?;
	mpv_events.observe_property(2, "pause")?;
	//mpv_events.observe_property(3, "playback-time")?;
	mpv_events.observe_property(4, "user-data/simulcast/fuckmpv")?;
	mpv_events.observe_property(5, "user-data/simulcast/input_reader")?;

	// let mut tick = 0;
	#[allow(non_snake_case)]
	let (mut A_spam_last, mut A_spam_count, mut A_spam_cooldown) =
		(std::time::SystemTime::now(), 0, std::time::SystemTime::UNIX_EPOCH);

	while let Ok(value) = mpv_events.listen_for_event() {
		//debug!("{}", value);
		match value["event"].as_str().unwrap() {
			"shutdown" => return Ok(()),
			"property-change" => {
				match value["name"].as_str().unwrap() {
					"pause" => {
						let paused = value["data"].as_bool().unwrap();
						let Ok(time) = mpv_query.get_property("playback-time/full") else {
							debug!("pause called. paused={paused}, no time though");
							continue;
						};
						let time = time.as_f64().unwrap();
						let mut state = state.lock().unwrap();

						debug!("pause called. state={}, new={}", state.paused, paused);

						if paused == state.paused {
							continue;
						}

						state.time = time;

						if state.party_count < 2 {
							state.paused = paused;
							continue;
						}

						debug!("about to do pause stuff. state={}, new={}", state.paused, paused);

						state.paused = true;
						drop(state);

						if paused {
							let _ = sender.send(WsMessage::AbsoluteSeek(time));
						} else {
							// if we are here then we probably unpaused with the onscreen-display
							mpv_query.set_property("pause", &json!(true))?;
							let _ = sender.send(WsMessage::Resume);
						}
					}
					"filename" => {
						let filename = value["data"].as_str().unwrap();
						let room_hash = {
							let mut state = state.lock().unwrap();
							if !state.room_code.is_empty() {
								// The roomid should:tm: still be valid.
								continue;
							} else {
								state.party_count = 0;
								if !filename.is_empty() {
									state.room_hash = get_room_hash(filename, &relay_room);
								}
							}
							state.room_hash.clone()
						};
						let _ = sender.send(WsMessage::Join(room_hash));
					}
					"user-data/simulcast/fuckmpv" => {
						let Some(data) = value["data"].as_str() else {
							// tf?
							continue;
						};

						if data == "." {
							continue;
						}

						debug!("user-data/simulcast/fuckmpv = '{data}'");
						mpv_query.set_property("user-data/simulcast/fuckmpv", &json!("."))?;

						if data == "queue_resume" {
							if state.lock().unwrap().party_count < 2 {
								mpv_query.set_property("pause", &json!(false))?;
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

							let _ = mpv_query.show_text(
								&format!("SIMULCAST\nparty count = {party_count}\ncustom room code = '{room_code}'\nroom id/hash = {room_hash}"),
								Some(7000),
								None
							);
						}
					}
					"user-data/simulcast/input_reader" => {
						let Some(data) = value["data"].as_str() else {
							// tf?
							continue;
						};
						let data = data.to_string();

						let room_hash = {
							let mut state = state.lock().unwrap();
							state.room_code = data;
							if !state.room_code.is_empty() {
								state.room_hash = get_room_hash(&state.room_code, &relay_room);
							} else {
								state.room_hash = get_room_hash(
									&mpv_query
										.get_property("filename")
										.map(|v| v.as_str().unwrap_or_default().to_string())
										.unwrap_or_else(|_| rand::random::<u64>().to_string()),
									&relay_room,
								);
							}
							state.room_hash.clone()
						};
						let _ = sender.send(WsMessage::Join(room_hash));
					}
					"playback-time" => {
						// tick += 1;
						// mpv.run_command(MpvCommand::ShowText {
						// 	text: tick.to_string(),
						// 	duration_ms: Some(100),
						// 	level: None,
						// })?;
					}
					_ => (),
				}
			}
			"seek" => {
				// This is dumb but necessary. We need *some* wait here otherwise it's desynced.
				// Related place to edit in server.rs. Ctrl+f "BROCCOLI".
				std::thread::sleep(Duration::from_millis(100));

				let time = mpv_query.get_property("playback-time/full")?.as_f64().unwrap();
				let paused = mpv_query.get_property("pause")?.as_bool().unwrap();
				let mut state = state.lock().unwrap();

				debug!("Event::Seek. time = {}. expected = {}", time, state.time);

				if (time - state.time).abs() > 0.03 {
					// seems like we seeked...

					state.time = time;
					let party_count = state.party_count;

					if party_count > 1 {
						state.paused = true;
					}

					drop(state);

					if party_count > 1 && !paused {
						mpv_query.set_property("pause", &json!(true))?;
					}

					if party_count > 1 {
						let _ = sender.send(WsMessage::AbsoluteSeek(time));
					}
				}
			}
			_ => (),
		}
	}

	Ok(())
}
