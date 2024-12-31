
# simulcast-mpv
A way to sync multiple [mpv players](https://mpv.io/) over the internet.
- If someone pauses/resumes, then pause/resume for everyone.
- If someone seeks forwards/backwards, then seek for everyone.
- Add in some ping calculation between clients.

That's basically what `simulcast-mpv` is.


## Usage
**TL;DR:**
- [Download](https://github.com/rtldg/simulcast-mpv/releases) `simulcast-mpv`
- Run `simulcast-mpv`. It will install itself.
- Start mpv. It should just work™.
- (optional) Hit `a` once to show some info. Hit `a` a few times really fast to open up an input window for custom room codes. (Maybe you and your friend are watching the same thing, but your file names are different.)

The `simulcast-mpv` executable has 4 "modes":
- `simulcast-mpv`
    - This "installs" `simulcast-mpv` to your mpv scripts directory (`%APPDATA%\mpv\scripts` or `~/.config/mpv/scripts`). It also writes a helper lua script (`simulcast-mpv.lua`) to the directory.
- `simulcast-mpv client`
    - This is ran when mpv starts. It acts as a middle-man for sending pause/resume/seek messages between mpv and the relay server.
- `simulcast-mpv relay`
    - A websocket server
- `simulcast-mpv input-reader`
    - A popup command prompt window for inputting custom room codes.


## **TODO:**
- Log simulcast-mpv things to mpv console.
- setup github actions to compile binaries for 64-bit ARM Linux (and also publish binaries to a release tag...).
    - `cargo +1.75 build --release` (1.75 for Windows 7 support)
    - `cargo zigbuild --release --target x86_64-unknown-linux-musl`
    - [cargo-dist](https://github.com/axodotdev/cargo-dist)? I don't particularly want to package .msi installers though...
- better logic to sync the position when someone joins a party (other than having an existing user just seek backwards from their position by 5s (which will sync the new user)...)
- Things don't always pause correctly when a party starts. Seems related to mpv automatically resuming/playing media when mpv opens.
- Fix some logic bug that gets you trapped in a pause/unpause loop.


## similar projects (for mpv)
- Syncplay: [website](https://syncplay.pl/) / [github](https://github.com/Syncplay/syncplay)
    - More feature-complete than `simulcast-mpv`.
- https://github.com/po5/groupwatch_sync
    - Manual groupwatch project.
- others? dunno...


## similar projects (not for mpv)
- Jellyfin has some "SyncPlay" thing.
- Plex (ew) has a "Watch Together" thing.
- [Metastream](https://github.com/samuelmaddock/metastream) does streaming & syncing on a webpage.


## Environment variables / .env
`simulcast-mpv` allows environment variables and files to configure some of the settings.

client
- `SIMULCAST_RELAY_URL` / `--relay-url` (default: reads the server from [here](https://github.com/rtldg/simulcast-mpv/blob/master/docs/servers.txt))
- `SIMULCAST_RELAY_ROOM` / `--relay-room` (default `abcd1234`)
- `SIMULCAST_CLIENT_SOCK` / `--client-sock` (passed by mpv to the simulcast-mpv executable)

relay server
- `SIMULCAST_BIND_ADDRESS` / `--bind-address` (default `127.0.0.1`)
- `SIMULCAST_BIND_PORT` / `--bind-port` (default `30777`)
- `SIMULCAST_REPO_URL` / `--repo-url` (for AGPL-3.0 reasons. Set this in your `.env` file if using 'docker compose')

Configuration files can be placed at
- `%APPDATA%\mpv\scripts\simulcast-mpv.env` (Windows)
- `~/.config/mpv/scripts/simulcast-mpv.env` (Unix)
- `$PWD/simulcast-mpv.env` (current directory AKA where mpv is started from) (Windows + Unix)


## Running the server (the intended way)
```sh
git clone https://github.com/rtldg/simulcast-mpv.git
cd simulcast-mpv
echo "SIMULCAST_REPO_URL=https://github.com/rtldg/simulcast-mpv" > .env
docker compose up -d

## then install caddy and reverse-proxy to 127.0.0.1:30777 like in this Caddyfile:
##  mydomain.com {
##    handle /simulcast-mpv {
##      reverse_proxy 127.0.0.1:30777
##    }
##  }

# To update:
git pull # update latest repo changes
docker compose build --no-cache simulcast-mpv-relay # rebuild or something lol... not sure if --no-cache is needed
docker compose down
docker compose up -d
```


## Relay server privacy
Relay server "rooms" are public to anyone who joins using the same "room ID".

"Room IDs" are calculated client-side as `blake3_hash(filename + relay_room)` where `relay_room` is configurable with `SIMULCAST_RELAY_ROOM`/`--relay-room`.

This means the server cannot know which file you are playing unless the server already knows what the `filename + relay_room` combination is.
