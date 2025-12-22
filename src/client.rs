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
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

use base64::prelude::*;

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
	custom_room_code: String,
	room_hash: String,
	room_random_chat_salt: String,
}

fn normalize_room_code(code: &str, relay_room: &str) -> String {
	code.chars()
		.map(|c| match c {
			'_' | '-' | '+' | '.' => ' ',
			_ => c,
		})
		.collect::<String>()
		+ relay_room
}

fn get_room_chat_key(code: &str, relay_room: &str, chat_salt: &str) -> [u8; 32] {
	let mut blah = normalize_room_code(code, relay_room);
	blah.push_str("chattychat");
	blah.push_str(chat_salt);
	*blake3::hash(blah.as_bytes()).as_bytes()
}

fn get_room_hash(code: &str, relay_room: &str) -> String {
	blake3::hash(normalize_room_code(code, relay_room).as_bytes())
		.to_hex()
		.to_string()
}

fn encrypt_chat(mut message: String, key: [u8; 32]) -> String {
	const PAD_TO: usize = 480;
	const PAD_SIZE: usize = 20;
	if message.len() < PAD_TO {
		message.reserve(PAD_TO - message.len());
	}
	while message.len() < PAD_TO - PAD_SIZE {
		message.push_str("                    ");
	}
	let key = aws_lc_rs::aead::RandomizedNonceKey::new(&aws_lc_rs::aead::AES_256_GCM, &key).unwrap();
	let mut in_out = message.into_bytes();
	let nonce = key
		.seal_in_place_append_tag(aws_lc_rs::aead::Aad::empty(), &mut in_out)
		.unwrap();
	in_out.extend_from_slice(nonce.as_ref());
	BASE64_STANDARD.encode(&in_out)
}

fn decrypt_chat(b64: &str, key: [u8; 32]) -> anyhow::Result<String> {
	let mut in_out = BASE64_STANDARD.decode(b64)?;
	let key = aws_lc_rs::aead::RandomizedNonceKey::new(&aws_lc_rs::aead::AES_256_GCM, &key).unwrap();
	let nonce_len = aws_lc_rs::aead::NONCE_LEN;
	anyhow::ensure!(in_out.len() > nonce_len + 2);
	let nonce = in_out.split_off(in_out.len() - nonce_len);
	let nonce = aws_lc_rs::aead::Nonce::try_assume_unique_for_key(&nonce).unwrap();
	let plaintext = key.open_in_place(nonce, aws_lc_rs::aead::Aad::empty(), &mut in_out)?;
	Ok(std::str::from_utf8(&plaintext)?.trim().to_owned())
}

async fn ws_thread(
	relay_url: String,
	mpv: &mut Mpv,
	receiver: &mut UnboundedReceiver<WsMessage>,
	state: Arc<Mutex<SharedState>>,
	relay_room: &str,
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

	ws.send(WsMessage::Info(String::from(env!("CARGO_PKG_VERSION"))).to_websocket_msg())
		.await?;
	ws.send(
		WsMessage::Info2 {
			version: env!("CARGO_PKG_VERSION").parse()?,
		}
		.to_websocket_msg(),
	)
	.await?;

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
					WsMessage::Chat(_) => {
						debug!("recv msg = Chat(<omitted>)");
					}
					WsMessage::RoomRandomChatSalt(_) => {
						debug!("recv msg = RoomRandomChatSalt(<omitted>)");
					}
					_ => debug!("recv msg = {msg:?}")
				}
				match msg {
					WsMessage::Info(s) => {
						info!("server info: {s}");
					},
					WsMessage::Info2 { version: _ } => {
						// nothing yet...
					}
					WsMessage::Join(_) => { /* we shouldn't be receiving this */ },
					WsMessage::Party(count) => {
						let (should_pause, should_seek) = {
							let mut state = state.lock().unwrap();

							// a new user has joined the party
							let should_seek = state.party_count > 0 && count > state.party_count;

							if state.party_count < 2 && count == 1 {
								// user is solo-watching and probably just opened mpv...
							} else {
								// party count has changed (or we just got a random Party msg?) so pause that bih
								state.paused = true;
							}

							state.party_count = count;
							(state.paused, should_seek)
						};

						let _ = mpv.set_property("user-data/simulcast/party_count", &json!(count));

						if should_pause {
							// these can hit too early and cause `Err(MpvError: property unavailable)`?
							let _ = mpv.set_property("pause", &json!(true));
							let _ = mpv.set_property("speed", &json!(1.0)); // useful for me (since I have my default mpv speed at 1.5x)

							let _ = mpv.show_text(&format!("party count: {count}"), Some(2000), None);
						}

						// TODO:
						// This isn't optimal because if every member sends a Seek (which they do)
						// then we could be jumping around. I don't feel like adding some
						// server-side hax to ignore all but the first seek. At least right now...
						// But that's probably the way to go.
						if should_seek {
							let Ok(time) = mpv.get_property("playback-time/full") else {
								continue;
							};
							let time = time.as_f64().unwrap();
							debug!("party_count increased so sending Seek");
							ws.send(WsMessage::AbsoluteSeek(time).to_websocket_msg()).await?;
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
					WsMessage::Chat(encrypted) => {
						let (code, chat_salt) = {
							let state = state.lock().unwrap();
							let code = if state.custom_room_code.is_empty() {
								state.room_code.clone()
							} else {
								state.custom_room_code.clone()
							};
							(code, state.room_random_chat_salt.clone())
						};

						let key = get_room_chat_key(&code, &relay_room, &chat_salt);

						let Ok(base_msg) = decrypt_chat(&encrypted, key) else {
							//debug!("");
							continue;
						};
						let base_msg = base_msg.trim();

						mpv.set_property("user-data/simulcast/latest-chat-message", &json!(base_msg))?;

						// "$>" disables 'Property Expansion' for `show-text`.  but it doesn't work here?
						let formatted_msg = format!(" \n \n \n \n \n \n \n \n \n \n \n \n> {}", base_msg);
						let _ = mpv.show_text(&formatted_msg, Some(5000), None);
					}
					WsMessage::RoomRandomChatSalt(salt) => {
						{
							let mut state = state.lock().unwrap();
							state.room_random_chat_salt = salt;
						}
					}
				}
			}
		}
	}
}

pub fn client(
	verbosity: log::LevelFilter,
	relay_url: Option<http::Uri>,
	relay_room: String,
	client_sock: String,
) -> anyhow::Result<()> {
	rustls::crypto::aws_lc_rs::default_provider().install_default().unwrap();

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
	let temp_directory = if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
		std::path::PathBuf::from(dir)
	} else {
		std::env::temp_dir()
	};

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
	.log_to_file(flexi_logger::FileSpec::default().directory(temp_directory))
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
			if mpv_heartbeat
				.set_property("user-data/simulcast/heartbeat", &json!(i))
				.is_err()
			{
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
		room_code: file.clone(),
		custom_room_code: String::new(),
		room_hash: get_room_hash(&file, &relay_room),
		room_random_chat_salt: String::new(),
	}));

	mpv_query.set_property("user-data/simulcast/party_count", &json!(0))?;
	mpv_query.set_property("user-data/simulcast/custom_room_code", &json!(""))?;
	mpv_query.set_property("user-data/simulcast/room_hash", &json!(state.lock().unwrap().room_hash))?;

	let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
	let state_ws = state.clone();
	let ws_relay_room = relay_room.clone();
	rt.spawn(async move {
		loop {
			let err = ws_thread(
				relay_url.to_string(),
				&mut mpv_ws,
				&mut receiver,
				state_ws.clone(),
				&ws_relay_room,
			)
			.await;
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
			tokio::time::sleep(Duration::from_secs_f64(std::f64::consts::PI)).await;
		}
	});

	mpv_events.observe_property(1, "filename")?;
	mpv_events.observe_property(2, "pause")?;
	//mpv_events.observe_property(3, "playback-time")?;
	mpv_events.observe_property(4, "user-data/simulcast/fuckmpv")?;
	mpv_events.observe_property(5, "user-data/simulcast/input_reader")?;
	mpv_events.observe_property(6, "user-data/simulcast/text_chat")?;

	// let mut tick = 0;
	let mut need_to_skip_first_unpause = true;

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

						if !paused && need_to_skip_first_unpause {
							need_to_skip_first_unpause = false;
							if state.party_count > 1 {
								drop(state);
								mpv_query.set_property("pause", &json!(true))?;
								continue;
							}
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
						let Some(filename) = value.get("data") else {
							continue;
						};
						let filename = filename.as_str().unwrap();

						let room_hash = {
							let mut state = state.lock().unwrap();
							if !state.custom_room_code.is_empty() {
								// The roomid should:tm: still be valid.
								continue;
							}
							state.party_count = 0;
							if !filename.is_empty() {
								state.room_code = filename.to_owned();
								state.room_hash = get_room_hash(filename, &relay_room);
							}
							state.room_hash.clone()
						};

						mpv_query.set_property("user-data/simulcast/party_count", &json!(0))?;
						mpv_query.set_property("user-data/simulcast/room_hash", &json!(room_hash))?;

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
						}
					}
					"user-data/simulcast/input_reader" => {
						let Some(data) = value["data"].as_str() else {
							// tf?
							continue;
						};
						let custom_room_code = data.to_string();

						let room_hash = {
							let mut state = state.lock().unwrap();
							state.custom_room_code = custom_room_code;
							if !state.custom_room_code.is_empty() {
								state.room_hash = get_room_hash(&state.custom_room_code, &relay_room);
							} else {
								state.room_hash = get_room_hash(&state.room_code, &relay_room);
							}
							state.room_hash.clone()
						};

						mpv_query.set_property("user-data/simulcast/custom_room_code", &json!(data))?;
						mpv_query.set_property("user-data/simulcast/room_hash", &json!(room_hash))?;

						let _ = sender.send(WsMessage::Join(room_hash));
					}
					"user-data/simulcast/text_chat" => {
						let Some(data) = value["data"].as_str() else {
							// tf?
							continue;
						};
						let text = data.to_string();

						let (code, chat_salt) = {
							let state = state.lock().unwrap();
							let code = if state.custom_room_code.is_empty() {
								state.room_code.clone()
							} else {
								state.custom_room_code.clone()
							};
							(code, state.room_random_chat_salt.clone())
						};

						let key = get_room_chat_key(&code, &relay_room, &chat_salt);

						let encrypted = encrypt_chat(text, key);

						let _ = sender.send(WsMessage::Chat(encrypted));
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

					mpv_query.set_property("user-data/simulcast/party_count", &json!(party_count))?;

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
