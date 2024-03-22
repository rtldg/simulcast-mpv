
# simulcast-mpv
I was curious how easy it would be to sync two mpv players across the internet, even though it's' overengineering a solution to a not-very problem.
If one person pauses, then pause for the other person. Add in some ping calculation between clients.
That's basically what `simulcast-mpv` is.

This isn't bug-free and I probably won't do anything to fix that.

## Usage
**TL;DR:**
- [Download](https://github.com/rtldg/simulcast-mpv/releases) `simulcast-mpv`
- Run `simulcast-mpv`. It will install itself.
- Start mpv. It should just work™.

The `simulcast-mpv` executable has 3 "modes":
- `simulcast-mpv`
    - This "installs" `simulcast-mpv` to your mpv scripts directory (`%APPDATA%\mpv\scripts` or `~/.config/mpv/scripts`). It also writes a helper lua script (`simulcast-mpv.lua`) to the directory.
- `simulcast-mpv client`
    - This is ran when mpv starts. It acts as a middle-man for sending pause/resume/seek messages between mpv and the relay server.
- `simulcast-mpv relay`
    - A websocket server

## **TODO:**
- setup github actions to compile binaries x86_64 Windows, x86_64 Linux, 64-bit ARM Linux.
- some logic bug somewhere for the pause/unpause on connect...

## similar projects (for mpv)
- Syncplay: [website](https://syncplay.pl/) / [github](https://github.com/Syncplay/syncplay)
    - More feature-complete than `simulcast-mpv`.
- https://github.com/po5/groupwatch_sync
    - Manual groupwatch project.
- others? dunno...

## Environment variables / .env
`simulcast-mpv` allows environment variables and files to configure some of the settings.

client
- `SIMULCAST_RELAY_URL` / `--relay-url` (default `wss://..../`)
- `SIMULCAST_RELAY_ROOM` / `--relay-room` (default `abcd1234`)
- `SIMULCAST_CLIENT_SOCK` / `--client-sock` (passed by mpv to the simulcast-mpv executable)
server
- `SIMULCAST_BIND_ADDRESS` / `--bind-address` (default `127.0.0.1`)
- `SIMULCAST_BIND_PORT` / `--bind-port` (default `30777`)

Configuration files can be placed at
- `%APPDATA%\mpv\scripts\simulcast-mpv.env` (Windows)
- `~/.config/mpv/scripts/simulcast-mpv.env` (Unix)
- `$PWD/simulcast-mpv.env` (current directory AKA where mpv is started from) (Windows + Unix)

## Relay server privacy
Relay server "rooms" are public to anyone who joins using the same "room ID".

"Room IDs" are calculated client-side as `blake3_hash(filename + relay_room)` where `relay_room` is configurable with `SIMULCAST_RELAY_ROOM`/`--relay-room`.

This means the server cannot know which file you are playing unless the server already knows what the `filename + relay_room` combination is.
