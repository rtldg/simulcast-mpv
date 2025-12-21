// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2025 rtldg <rtldg@protonmail.com>

use crate::message::WsMessage;
use chrono::prelude::*;
use futures::{SinkExt, StreamExt};
use log::{debug, info};
use std::{
	borrow::BorrowMut,
	collections::HashMap,
	ops::DerefMut,
	sync::{Arc, Mutex},
	time::Duration,
};

use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

struct Member {
	id: u64,
	ping: f64,
	version: semver::Version,
	sender: tokio::sync::mpsc::UnboundedSender<Message>,
}

#[derive(Default)]
struct Room {
	queued_resumes: Option<tokio::task::JoinSet<()>>,
	members: Vec<Member>,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

static REPO_URL: std::sync::OnceLock<http::Uri> = std::sync::OnceLock::new();

fn remove_from_room(id: u64, current_room: &String, rooms: &mut HashMap<String, Room>) -> Member {
	let members = &mut rooms.get_mut(current_room).unwrap().members;
	let i = members.iter().position(|m| m.id == id).unwrap();
	let me = members.swap_remove(i);
	if members.is_empty() {
		rooms.remove(current_room);
	} else {
		let len = members.len();
		let msg = WsMessage::Party(len as u32).send_helper();
		for member in members {
			let _ = member.sender.send(msg.clone());
		}
	}
	me
}

async fn handle_websocket(
	stream: tokio::net::TcpStream,
	id: u64,
	addr: std::net::SocketAddr,
	rooms: Rooms,
	connected_counter: Arc<()>,
) -> anyhow::Result<()> {
	let mut current_room = String::new();
	let ret = handle_websocket_inner(stream, id, &mut current_room, rooms.clone()).await;
	if current_room != "" {
		let mut rooms = rooms.lock().unwrap();
		let _ = remove_from_room(id, &current_room, rooms.deref_mut());
	}
	let num_connected = Arc::strong_count(&connected_counter) - 2; // -1 for ourself & -1 for the original
	info!("finished with client {id} {addr} ({num_connected} clients connected) {ret:?}");
	ret
}

async fn handle_websocket_inner(
	stream: tokio::net::TcpStream,
	id: u64,
	current_room: &mut String,
	rooms: Rooms,
) -> anyhow::Result<()> {
	let ws = tokio_tungstenite::accept_async_with_config(
		stream,
		Some(
			WebSocketConfig::default()
				.max_message_size(Some(1024))
				.max_frame_size(Some(800))
				.accept_unmasked_frames(false),
		),
	)
	.await?;

	// We still want ping calculation even when a user isn't in a room...
	let mut ping = 0.0;

	let mut client_version = semver::Version::parse("2.0.0").unwrap();

	let (mut ws_s, mut ws_r) = ws.split();
	let (ch_s, mut ch_r) = tokio::sync::mpsc::unbounded_channel();

	tokio::spawn(async move {
		while let Some(msg) = ch_r.recv().await {
			let _ = ws_s.send(msg).await;
		}
	});

	// Using an `Instant` instead of `intervals_since_last_pong` because it's less prone to breaking in case the interval duration is ever changed for some reason.
	let mut last_pong_time = std::time::Instant::now();

	let mut interval = tokio::time::interval(Duration::from_secs(1));
	loop {
		tokio::select! {
			_ = interval.tick() => {
				if last_pong_time.elapsed() > Duration::from_secs(10) {
					anyhow::bail!("client {id} hasn't pong'd for 10s and probably lost connection."); // anyhow::bail!() will return btw...
				}

				let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
				ch_s.send(WsMessage::Ping(now).send_helper())?;
			}
			msg = ws_r.next() => {
				let Some(msg) = msg else { return Ok(()); };
				let msg = msg?.into_text()?;
				let Ok(msg) = serde_json::from_str(&msg) else {
					//debug!("unknown message from client {id} msg = {msg}");
					continue;
				};
				match msg {
					WsMessage::Ping(_) | WsMessage::Pong(_) => (),
					_ => debug!("recv msg = {msg:?}")
				}
				match msg {
					WsMessage::Info(_) => {
						// Could be a more strongly-typed info message via json+serde but it doesn't really matter.
						let s = format!("version {} repo {}", env!("CARGO_PKG_VERSION"), REPO_URL.get().unwrap());
						let _ = ch_s.send(WsMessage::Info(s).send_helper());
					}
					WsMessage::Info2 { version } => {
						client_version = version;
					}
					WsMessage::Join(ref new_room) => {
						if new_room.as_str() == current_room {
							continue;
						}

						let mut rooms = rooms.lock().unwrap();

						let me = if current_room == "" {
							Member {
								id,
								ping,
								version: client_version.clone(),
								sender: ch_s.clone(),
							}
						} else {
							remove_from_room(id, current_room, rooms.deref_mut())
						};

						if new_room != "" {
							let room = rooms.entry(new_room.clone()).or_default();
							room.members.push(me);
							let len = room.members.len();
							let msg = WsMessage::Party(len as u32).send_helper();
							for member in &room.members {
								let _ = member.sender.send(msg.clone());
							}
						}

						current_room.clone_from(new_room);
					}
					WsMessage::Party(_) => { /* we shouldn't be receiving this */ }
					WsMessage::Resume => {
						if current_room == "" {
							continue;
						}

						let msg = WsMessage::Resume.send_helper();

						let mut rooms = rooms.lock().unwrap();
						let room = rooms.get_mut(current_room).unwrap();

						// We can reach this with pause mismatches and shit...
						if let Some(queued) = room.queued_resumes.borrow_mut() {
							while queued.try_join_next().is_some() {}
							if queued.is_empty() {
								room.queued_resumes = None;
							}
						}

						// An existing queue is occuring and we probably shouldn't hit this but...
						if room.queued_resumes.is_some() {
							continue;
						}

						let highest_ping = room
							.members
							.iter()
							.map(|m| m.ping)
							.max_by(|a, b| a.total_cmp(b))
							.unwrap();

						let mut set = tokio::task::JoinSet::new();
						for member in &room.members {
							// let id = member.id;
							let sender = member.sender.clone();
							let delay = Duration::from_secs_f64(highest_ping - member.ping);
							let msg = msg.clone();
							set.spawn(async move {
								if !delay.is_zero() {
									tokio::time::sleep(delay).await;
								}
								let _ = sender.send(msg);
							});
						}
						room.queued_resumes = Some(set);
					}
					WsMessage::AbsoluteSeek(t) => {
						if current_room == "" {
							continue;
						}

						let msg = WsMessage::AbsoluteSeek(t).send_helper();

						let mut rooms = rooms.lock().unwrap();
						let room = rooms.get_mut(current_room).unwrap();
						drop(room.queued_resumes.take()); // abort queued resumes...

						for member in &room.members {
							// NOTE: We might need to send the seek to the same user that sent the seek.
							// It can be a bit desynced if we don't...
							// It depends on if we have a sleep in the Event::Seek though... BROCCOLI
							if member.id != id {
								let _ = member.sender.send(msg.clone());
							}
						}
					}
					WsMessage::Ping(_) => { /* we shouldn't be recieving this */ }
					WsMessage::Pong(ref s) => {
						let elapsed = Utc::now()
							.signed_duration_since(DateTime::parse_from_rfc3339(s)?)
							.to_std()?
							.as_secs_f64();
						ping = elapsed / 2.0;
						//debug!("  ping = {ping}s");

						last_pong_time = std::time::Instant::now();

						if current_room != "" {
							let mut rooms = rooms.lock().unwrap();
							let room = rooms.get_mut(current_room).unwrap();
							room.members.iter_mut().find(|m| m.id == id).unwrap().ping = ping;
						}
					}
					WsMessage::Chat(encrypted) => {
						if current_room == "" {
							continue;
						}

						let msg = WsMessage::Chat(encrypted).send_helper();

						const CHAT_MIN_VERSION: semver::Version = semver::Version {
							major: 2,
							minor: 3,
							patch: 0,
							pre: semver::Prerelease::EMPTY,
							build: semver::BuildMetadata::EMPTY,
						};

						let mut rooms = rooms.lock().unwrap();
						let room = rooms.get_mut(current_room).unwrap();

						for member in &room.members {
							if member.version >= CHAT_MIN_VERSION {
								let _ = member.sender.send(msg.clone());
							}
						}
					}
				}
			}
		}
	}
}

async fn async_server(addr: std::net::SocketAddr) -> anyhow::Result<()> {
	let listener = tokio::net::TcpListener::bind(addr).await?;
	info!("listening on {addr}");

	let rooms: Rooms = Default::default();
	let mut latest_id = 0;
	let connected_counter = Arc::new(());

	loop {
		if let Ok((stream, addr)) = listener.accept().await {
			latest_id += 1;
			let rooms = rooms.clone();
			let num_connected = Arc::strong_count(&connected_counter);
			info!("accepted client {latest_id} {addr} ({num_connected} clients connected)");
			tokio::spawn(handle_websocket(
				stream,
				latest_id,
				addr,
				rooms,
				connected_counter.clone(),
			));
		}
	}
}

pub fn server(
	verbosity: log::LevelFilter,
	bind_address: std::net::IpAddr,
	bind_port: u16,
	repo_url: &http::Uri,
) -> anyhow::Result<()> {
	flexi_logger::Logger::with(
		flexi_logger::LogSpecification::builder()
			.default(verbosity)
			.module("rustls", log::LevelFilter::Warn)
			.module("tokio_tungstenite", log::LevelFilter::Warn)
			.module("tungstenite", log::LevelFilter::Warn)
			.build(),
	)
	.format(flexi_logger::colored_default_format)
	.start()?;

	let _ = REPO_URL.get_or_init(|| repo_url.clone());
	let addr = std::net::SocketAddr::new(bind_address, bind_port);
	let rt = tokio::runtime::Runtime::new()?;
	rt.block_on(async move { async_server(addr).await })
}
