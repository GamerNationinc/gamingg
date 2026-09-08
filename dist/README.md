# gamingg — Linux build

A ready-to-run build of the game, for testing on a Steam Deck or any x86-64
Linux desktop.

```
gamingg-linux-x86_64    the game, stripped, ~10 MB
SHA256SUMS              so you can tell one build from the next
install-steamdeck.sh    one-command install: verify, ~/.local, launcher
```

There is one binary here and it is overwritten each round rather than
accumulating — ten megabytes a build adds up fast in a git pack. If handing out
builds becomes routine this should move to GitHub Releases instead of living in
the tree.

---

## Check this first — thirty seconds

The build needs **glibc 2.39 or newer**. Rust's standard library pulls in
`pidfd_spawnp@GLIBC_2.39`, and that version requirement is not marked weak, so
the loader refuses the binary outright on anything older rather than degrading
gracefully.

```bash
ldd --version | head -1
```

- **SteamOS 3.7 and up** is Arch-based with glibc 2.41. Fine.
- **SteamOS 3.6** shipped glibc 2.37. This build will not start, and the error
  will be `version 'GLIBC_2.39' not found`.

If you are on the older one, that needs a rebuild against an older glibc — a
different build container, not something that can be patched here. Say so and it
gets done next round.

## Running it

```bash
# Deck, Desktop Mode, in Konsole:
git clone <this repo> && cd gamingg
./dist/gamingg-linux-x86_64          # run in place, or:
cd dist && ./install-steamdeck.sh    # install + menu launcher + Steam notes
```

The installer checks the binary against `SHA256SUMS`, installs to
`~/.local/share/gamingg/`, drops a launcher in the applications menu, and
prints the *Add a Non-Steam Game* steps that make it appear in Gaming
Mode's library. `--uninstall` reverses it; saves stay.

`git` preserves the executable bit, so a clone needs no `chmod`. A file
downloaded through a browser does: `chmod +x gamingg-linux-x86_64`.

**The Deck's own controls drive it now.** The game reads pads natively
(left stick moves, right stick looks, `RT` drills, `SELECT` shows the whole
scheme in game), and Steam Input's default Gamepad layout is exactly what
it expects. Every control the game has is on the pad somewhere: hold `LB`
for the block palette, hold `SELECT` for the second layer, and `B` hangs up
a machine feed. Typing into the terminal is done from an on-screen keyboard the pad drives,
so a keyboard is not wanted for anything. A keyboard is only needed for *typing* into the terminal —
everything else, panels included, answers to the pad.

## What it needs from the system

Almost nothing. Both shaders are compiled into the binary and there is no asset
directory, so this one file is the whole game — every sound is synthesized at
runtime, so there are no audio files either. It links `libc`, `libm`,
`libgcc_s`, (since the arsenal round) `libasound.so.2` for sound, and (since
the gamepad round) `libudev.so.1` to enumerate controllers — all of which
SteamOS and every desktop distribution ship; X11, Wayland and Vulkan are all
opened at runtime, so it uses whatever the machine has. A machine with no
audio device just plays silently. The Deck's AMD GPU runs the RADV Vulkan
driver natively and needs nothing installed.

Worlds and settings follow the XDG spec:

```
~/.local/share/gamingg/saves     worlds
~/.config/gamingg                settings
```

Both are under your home directory, which matters on SteamOS — the root
filesystem is read-only, so the game must never be run from a directory it
expects to write to.

## Useful flags

```bash
./gamingg-linux-x86_64 --help              every option
./gamingg-linux-x86_64 --seed 12345        a different world
./gamingg-linux-x86_64 --third-person      start over the shoulder
./gamingg-linux-x86_64 --screenshot out.ppm --at 40,40
```

## Controls

Pad: left stick moves (click sprints), right stick looks (click for optics),
`RT` drill/fire, `LT` place, `A` jump/confirm, `B` crouch/back, `X` use,
`Y` handheld, `LB` camera, `RB` page/method, d-pad mark/minimap/scan/fly,
`Start` confirm, `SELECT` the scheme on screen.

Keyboard: `WASD` move, `Space` jump, `Left Shift` crouch, `Left Ctrl` sprint,
`Left Ctrl`+`Left Shift` slide, `Z` prone, `C` first/third person, `E` interact,
`V` handheld, `M` mark ore, `Tab` cycle mining method, `7` the slug launcher
once the shop has sold you one. Waist-high ledges are vaulted automatically —
you do not press anything for those.

The full list is in the main `README.md`.

## If it does not start

| What you see | What it is |
|---|---|
| `version 'GLIBC_2.39' not found` | SteamOS too old — see the check at the top |
| `Permission denied` | `chmod +x gamingg-linux-x86_64` |
| No window, Vulkan errors | The GPU driver is not being found; run it from Desktop Mode rather than through Gaming Mode's overlay |

## Verifying the download

```bash
sha256sum -c SHA256SUMS
```
