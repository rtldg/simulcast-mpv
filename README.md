
# simulcast-mpv
I was curious how easy it would be to sync two mpv players across the internet, even though it's' overengineering a solution to a not-very problem.
If one person pauses, then pause for the other person. Add in some ping calculation between clients.
That's basically what simulcast-mpv is.

This isn't bug-free and I probably won't do anything to fix that.

### **TODO:**
- setup public relay server (fly.io?)
- precompiled binaries for x86-64 Windows, x86-64 Linux, 64-bit ARM Linux.
- some logic bug somewhere for the pause/unpause on connect...
- Requires [this](https://github.com/rtldg/mpvipc) crate at `../mpvipc` to work. It will be added as a pinned github url eventually.

### similar projects (for mpv)
- Syncplay: [website](https://syncplay.pl/) / [github](https://github.com/Syncplay/syncplay)
    - More feature-complete than simulcast-mpv.
- https://github.com/po5/groupwatch_sync
    - Manual groupwatch project.
- others? dunno...
