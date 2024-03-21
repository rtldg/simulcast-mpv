// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2024 rtldg <rtldg@protonmail.com>

// https://github.com/snapview/tungstenite-rs

// https://crates.io/crates/shadow-rs
//    A build-time information stored in your rust project

// https://mpv.io/manual/master/
// https://github.dev/mpv-player/mpv
// https://docs.rs/mpvipc/

// cargo run --release -- client --client-sock mpvsock42

// cargo +1.75 build --release
// rustup override set 1.75 # last version before win7 support died

use anyhow::Context;
use log::debug;
use log::error;
use log::info;
use mpvipc::MpvDataType;
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
}

fn room_id(mpv: &Mpv, relay_room: &str) -> Result<String, mpvipc::Error> {
	let title_or_file = mpv.get_property_string("filename")? + relay_room;
	// We don't want the server to see media titles...
	Ok(blake3::hash(title_or_file.as_bytes()).to_hex().to_string())
}

async fn ws_thread(
	relay_url: String,
	relay_room: &str,
	mpv: &Mpv,
	receiver: &mut UnboundedReceiver<WsMessage>,
	state: Arc<Mutex<SharedState>>,
) -> anyhow::Result<()> {
	info!("ws_thread!");

	let (mut ws, _) = tokio_tungstenite::connect_async(relay_url)
		.await
		.context("Failed to setup websocket connection")?;

	info!("connected to websocket");

	ws.send(Message::Text(
		serde_json::to_string(&WsMessage::Join(room_id(mpv, relay_room)?)).unwrap(),
	))
	.await?;

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
								duration_ms: Some(1000),
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

pub fn client(
	verbosity: log::LevelFilter,
	relay_url: http::Uri,
	relay_room: String,
	client_sock: String,
) -> anyhow::Result<()> {
	let verbosity = log::LevelFilter::Debug;
	flexi_logger::Logger::with(
		flexi_logger::LogSpecification::builder()
			.default(verbosity)
			.module("mpvipc", log::LevelFilter::Error)
			.build(),
	)
	.format(flexi_logger::detailed_format)
	.log_to_stdout()
	.log_to_file(flexi_logger::FileSpec::default())
	// .log_to_file(flexi_logger::FileSpec::try_from("simulcast.log")?)
	.start()?;
	// simple_logging::log_to_file("out.log", verbosity)?;

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
	let mut mpv = Mpv::connect(&client_sock)?;
	let mpv_ws = Mpv::connect(&client_sock)?;

	info!("mpv objects setup...");

	let _ = std::thread::spawn(move || {
		let mpv_heartbeat = Mpv::connect(&client_sock).unwrap();
		for i in 1..usize::MAX {
			std::thread::sleep(Duration::from_secs_f64(0.1));
			mpv_heartbeat.set_property("user-data/simulcast/heartbeat", i).unwrap();
		}
	});

	let state = Arc::new(Mutex::new(SharedState {
		party_count: 0,
		paused: false,
		time: 0.0,
	}));
	let state_ws = state.clone();

	let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();

	let rt = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.worker_threads(2)
		.build()?;
	let relay_room_clone = relay_room.to_owned();
	rt.spawn(async move {
		loop {
			let state = state_ws.clone();
			let err = ws_thread(relay_url.to_string(), &relay_room_clone, &mpv_ws, &mut receiver, state).await;
			if let Err(err) = err {
				error!("{:?}", err);
			}
		}
	});

	let track: String = mpv.get_property("filename")?;
	info!("track = '{track}'");

	mpv.observe_property(1, "filename")?;
	mpv.observe_property(2, "pause")?;
	mpv.observe_property(3, "playback-time")?;
	mpv.observe_property(4, "user-data/simulcast/fuckmpv")?;

	// let mut tick = 0;

	loop {
		match mpv.event_listen()? {
			Event::Shutdown => return Ok(()),
			Event::Unimplemented => {}
			Event::PropertyChange { id: _, property } => match property {
				Property::Pause(paused) => {
					info!("pause called");

					let Ok(time) = mpv.get_property("playback-time/full") else {
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
						mpv.set_property("pause", true)?;
						let _ = sender.send(WsMessage::Resume);
					}
				}
				Property::Unknown { name, data } => match name.as_str() {
					"filename" => {
						let _ = sender.send(WsMessage::Join(room_id(&mpv, &relay_room)?));
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
						mpv.set_property("user-data/simulcast/fuckmpv", ".".to_string())?;

						if data == "queue_resume" {
							if state.lock().unwrap().party_count < 2 {
								mpv.set_property("pause", false)?;
								continue;
							}

							// let time: f64 = mpv.get_property("playback-time/full")?;
							// sender.send(WsMessage::AbsoluteSeek(time))?;
							let _ = sender.send(WsMessage::Resume);
						}
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

				let time: f64 = mpv.get_property("playback-time/full")?;
				let paused: bool = mpv.get_property("pause")?;
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
						mpv.set_property("pause", true)?;
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
