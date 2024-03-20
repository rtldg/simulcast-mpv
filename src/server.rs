// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2024 rtldg <rtldg@protonmail.com>

use crate::message::WsMessage;
use chrono::prelude::*;
use futures::{SinkExt, StreamExt};
use std::{
	borrow::BorrowMut,
	collections::HashMap,
	ops::DerefMut,
	sync::{atomic::AtomicU64, Arc, Mutex},
	time::Duration,
};

use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

struct Member {
	id: u64,
	ping: f64,
	sender: tokio::sync::mpsc::UnboundedSender<WsMessage>,
}

#[derive(Default)]
struct Room {
	queued_resumes: Option<tokio::task::JoinSet<()>>,
	members: Vec<Member>,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

fn remove_from_room(id: u64, current_room: &String, rooms: &mut HashMap<String, Room>) -> Member {
	let members = &mut rooms.get_mut(current_room).unwrap().members;
	let i = members.iter().position(|m| m.id == id).unwrap();
	let me = members.swap_remove(i);
	if members.is_empty() {
		rooms.remove(current_room);
	} else {
		let len = members.len();
		for member in members {
			let _ = member.sender.send(WsMessage::Party(len as u32));
		}
	}
	me
}

async fn handle_websocket(
	stream: tokio::net::TcpStream,
	id: u64,
	addr: std::net::SocketAddr,
	rooms: Rooms,
) -> anyhow::Result<()> {
	let mut current_room = String::new();
	let ret = handle_websocket_inner(stream, id, &mut current_room, rooms.clone()).await;
	if current_room != "" {
		let mut rooms = rooms.lock().unwrap();
		let _ = remove_from_room(id, &current_room, rooms.deref_mut());
	}
	println!("finished with client {id} {addr}");
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
		Some(WebSocketConfig {
			max_send_queue: None,
			max_message_size: Some(512),
			max_frame_size: Some(800),
			accept_unmasked_frames: false,
		}),
	)
	.await?;

	// We still want ping calculation even when a user isn't in a room...
	let mut ping = 0.0;

	let (mut ws_s, mut ws_r) = ws.split();
	let (ch_s, mut ch_r) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();

	tokio::spawn(async move {
		while let Some(msg) = ch_r.recv().await {
			// println!("send msg = {msg:?}");
			let _ = ws_s.send(Message::Text(serde_json::to_string(&msg).unwrap())).await;
		}
	});

	let mut interval = tokio::time::interval(Duration::from_secs(1));
	loop {
		tokio::select! {
			_ = interval.tick() => {
				let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
				ch_s.send(WsMessage::Ping(now))?;
			}
			msg = ws_r.next() => {
				let Some(msg) = msg else { return Ok(()); };
				let msg: WsMessage = serde_json::from_str(&msg?.into_text()?)?;
				// println!("recv msg = {msg:?}");
				match msg {
					WsMessage::Join(ref new_room) => {
						if new_room.as_str() == current_room {
							continue;
						}

						let mut rooms = rooms.lock().unwrap();

						let me = if current_room == "" {
							Member {
								id,
								ping,
								sender: ch_s.clone(),
							}
						} else {
							remove_from_room(id, current_room, rooms.deref_mut())
						};

						if new_room != "" {
							let room = rooms.entry(new_room.clone()).or_default();
							room.members.push(me);
							let len = room.members.len();
							for member in &room.members {
								let _ = member.sender.send(WsMessage::Party(len as u32));
							}
						}

						current_room.clone_from(new_room);
					}
					WsMessage::Party(_) => { /* we shouldn't be receiving this */ }
					WsMessage::Resume => {
						if current_room == "" {
							continue;
						}

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
							set.spawn(async move {
								if !delay.is_zero() {
									tokio::time::sleep(delay).await;
								}
								// println!("sent resume to {}", id);
								let _ = sender.send(WsMessage::Resume);
							});
						}
						room.queued_resumes = Some(set);
					}
					WsMessage::AbsoluteSeek(t) => {
						if current_room == "" {
							continue;
						}

						let mut rooms = rooms.lock().unwrap();
						let room = rooms.get_mut(current_room).unwrap();
						drop(room.queued_resumes.take()); // abort queued resumes...

						for member in &room.members {
							let _ = member.sender.send(WsMessage::AbsoluteSeek(t));
						}
					}
					WsMessage::Ping(_) => { /* we shouldn't be recieving this */ }
					WsMessage::Pong(ref s) => {
						let elapsed = Utc::now()
							.signed_duration_since(DateTime::parse_from_rfc3339(s)?)
							.to_std()?
							.as_secs_f64();
						ping = elapsed / 2.0;
						// println!("  ping = {ping}s");

						if current_room != "" {
							let mut rooms = rooms.lock().unwrap();
							let room = rooms.get_mut(current_room).unwrap();
							room.members.iter_mut().find(|m| m.id == id).unwrap().ping = ping;
						}
					}
				}
			}
		}
	}
}

async fn async_server(addr: std::net::SocketAddr) -> anyhow::Result<()> {
	let listener = tokio::net::TcpListener::bind(addr).await?;
	println!("listening on {addr}");

	let rooms = Arc::new(Mutex::new(HashMap::<String, Room>::new()));
	let latest_id = AtomicU64::new(1);

	loop {
		if let Ok((stream, addr)) = listener.accept().await {
			let id = latest_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			let rooms = rooms.clone();
			println!("accepted client {id} {addr}");
			tokio::spawn(handle_websocket(stream, id, addr, rooms));
		}
	}
}

pub fn server(verbosity: log::LevelFilter, bind_address: std::net::IpAddr, bind_port: u16) -> anyhow::Result<()> {
	let addr = std::net::SocketAddr::new(bind_address, bind_port);
	let rt = tokio::runtime::Runtime::new()?;
	rt.block_on(async move { async_server(addr).await })
}
