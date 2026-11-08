# StrataMon

A free, open-source desktop monitor for your [Strata](https://github.com/Niko1221/Strata) servers.
Strata already reports on itself, and StrataMon shows those reports for all your servers in one
window. Nothing gets installed on the servers.

StrataMon is an independent project, and isn't part of Strata.

For each server, it shows the same things as Strata's own Monitor:

- What the model is doing, with progress through the current request
- The model's name and context size, and how its KV cache is quantized (`q8_0`, `int8`, `q4_k_m`, ...)
- Speed, GPU load, VRAM, GPU temperature, power, PCIe, CPU and disk, each with a six-minute trend
- Context fill, system RAM and experts in VRAM
- Requests running and waiting, and the most recent requests

It also raises an alert when a server needs attention, and lists everything it notices in an events
feed along the bottom of the window. See [Alerts](#alerts).

> **Status: rough draft.** It's ready for testing, not yet for relying on.

## Getting started

### 1. Get the app

StrataMon is one 1.2 MB file with nothing to install, and it uses about 30 MB of memory. A ready-built
copy is in `dist\StrataMon-win-x64.exe` — copy it anywhere and run it on 64-bit Windows 10 or later.
It doesn't need anything else installed, it unpacks nothing, and you can move it anywhere.

StrataMon runs on Windows only for now. **Linux support is planned.**

To build it yourself, see [Development](#development).

### 2. Let it reach Strata

Strata only answers on its own PC until you open it up. How you do that depends on how you run it.

| How you run Strata | What to do |
|---|---|
| **Terminal** (`strata launch`) | At the Access step, choose **Tailscale** or **My local network**, or pass `--access tailscale`. Keep the API key it makes for you. |
| **Source checkout** (`START-HERE.bat`) | Add `"host": "0.0.0.0"` and `"api_key": "some-long-secret"` to `strata-<model>.json`. |
| **Desktop app** | See [below](#the-strata-desktop-app). |
| **On the same PC as StrataMon** | Nothing. Use the address `127.0.0.1`. |

On Windows, the firewall also has to allow the port. If Windows doesn't ask when Strata starts, run
this in an administrator PowerShell:

```powershell
New-NetFirewallRule -DisplayName "Strata 8080" -Direction Inbound -Protocol TCP -LocalPort 8080 -Action Allow -Profile Private
```

### 3. Add your servers

Open **Settings** and add each server with a name, its LAN address, its Tailscale address if it has
one, and its key. **Test connection** checks each address separately.

Addresses use port 8080 unless you add another, like `192.168.1.10:8095`. Prefer IP addresses,
including Tailscale's `100.x.y.z` ones: without an API key, Strata refuses names it doesn't know.

### The Strata desktop app

The desktop app always keeps its engine on its own PC, and its **Share with other devices** port only
carries chat, so StrataMon can't reach the monitoring through either one.

If the PC is on Tailscale, you can share the engine with your tailnet instead. Run this on that PC,
changing 8080 if the app uses another port:

```powershell
tailscale serve --bg --http=8081 http://127.0.0.1:8080
```

Then use that PC's `100.x.y.z:8081` as its Tailscale address in StrataMon. This shares the whole
engine with your tailnet, not only its monitoring, so set an API key if other people use your tailnet.
`tailscale serve reset` undoes it, along with anything else you've shared this way on that PC.

## Which key to use

| Key | What you see |
|---|---|
| Strata's API key, or none if the server has none | Everything |
| A read-only telemetry key | Everything except the KV cache type, CPU, disk, PCIe and expert readings, and the alert for a model that's unavailable |

Strata's Terminal monitor shows a server's API key on its Connect view (press `7`). It's also the
`"api_key"` value in `strata-<model>.json` on that server.

A telemetry key can't run the model or change anything, so it's safer to leave on a machine. Set one
with `"telemetry_read_key"` in `strata-<model>.json`, or `STRATA_TELEMETRY_KEY`. It has to differ from
the API key.

## How it works

```
StrataMon  ── polls every 3 seconds ──▶  Strata on each server
```

StrataMon reads two things Strata publishes about itself, and works with either one:

| Endpoint | What it adds | Needs |
|---|---|---|
| `/metrics` | Everything on Strata's Monitor: state, speed, hardware, recent requests | The API key, if the server has one |
| `/v1/telemetry` | Restarts, shutting down, and requests rejected or timed out by the queue limits | Either key, if the server has one |

Older versions of Strata have no `/v1/telemetry`, so queue limits go unreported there, but the rest works.

A server can have a LAN address and a Tailscale address, and the app uses whichever one answers, so
it keeps working when you leave home.

## Alerts

| Kind | When |
|---|---|
| **Critical** | The PC goes offline, Strata stops answering, a key or address is refused, the model is unavailable, or Strata is shutting down |
| **Over limit** | GPU temperature or system RAM stays over its limit, or requests stay queued too long |
| **Notice** | Strata restarted, or requests failed or were turned away |

You can change the limits in **Settings**. A reading has to stay over its limit for a minute before it
alerts, so brief spikes don't. Two things never alert, because they're normal for Strata: full VRAM,
which it fills on purpose, and an unloaded model, since it can unload when idle.

Everything that fires, plus recoveries, lands in the **Events** feed along the bottom of the window,
newest first. Scroll inside it to see older rows, and drag its top edge to make it taller or shorter.
It keeps the last 50 events since StrataMon started.

## Updates and privacy

StrataMon only contacts the servers you add. It doesn't check for updates at launch or on a timer,
and it has no telemetry, analytics or crash reporting.

> **Updates aren't switched on yet.** This build has no release feed or release key set, so
> **Check for updates** only says that updates aren't set up, and it doesn't contact anything.
> Until they are, get new builds from wherever you got this one, or build your own as described
> under [Development](#development).

This signature is separate from Windows code signing. The first time you run a new copy, Windows
SmartScreen may warn that it doesn't recognize the app.

## Security

- **It's read-only.** StrataMon only sends GET requests to `/health`, `/metrics` and
  `/v1/telemetry`. It doesn't read prompts or answers, and it can't change anything on a server.
- **Your key only goes to Strata.** Before every poll, StrataMon asks the address what it is, without
  sending the key. A device that happens to have that LAN address on another network never receives
  it. This stops accidents, not attackers: a device that pretends to be Strata could still get the key.
- **Traffic isn't encrypted on your LAN.** Strata serves plain HTTP, so your key and stats cross the
  LAN as readable text. Tailscale encrypts its traffic, so prefer Tailscale addresses when you can.
  Addresses that start with `https://` also work, if you've put HTTPS in front of Strata. Without a
  port they still use Strata's 8080, so add `:443` if that's where HTTPS answers.
- **Opening Strata opens the model too.** Anyone who can reach the port can use it, so set an API key.
  Don't expose it to the internet.
- **Keys are sealed** in `%APPDATA%\StrataMon\config.json`, locked to your Windows account: the
  file opens nowhere else — another PC, another account, a copied backup — and its contents are
  scrambled to anyone who looks at it. If your Windows password is reset by someone other than you,
  the seal closes too, and StrataMon keeps the file and starts fresh. Settings from an older install
  are copied here on first run and sealed in the same run. The old folder stays, so delete it once
  you no longer need it.

## Troubleshooting

**Not running.** The PC answered, but nothing is listening on that port. Strata has stopped, or it's
still set to answer only on its own PC. See [step 2](#2-let-it-reach-strata).

**Offline.** The PC didn't answer. It's off, the network is down, or a firewall is dropping the port.

**Needs a key.** This Strata has an API key, so StrataMon needs it too. See
[Which key to use](#which-key-to-use).

**Key rejected.** The key is wrong.

**Name refused.** Strata doesn't know this name. Use its IP address, add the name to `"allowed_hosts"`
in `strata-<model>.json`, or set an API key.

**Needs update.** This Strata has no monitoring endpoints, or reports them in a format this version of
StrataMon can't read. Update whichever is older.

**CPU, disk and PCIe show n/a.** You're using a read-only key. See [Which key to use](#which-key-to-use).

**Going offline takes a while to show.** A server counts as offline after three failed polls in a row,
so one dropped packet doesn't alert. That takes about 10 to 20 seconds.

## Development

StrataMon is written in Rust. Building needs [Rust](https://rustup.rs/) and the MSVC build tools.

```powershell
cargo test --workspace
cargo run -p stratamon -- --config .\config.json
./scripts/publish.ps1
```

`publish.ps1` builds `dist\StrataMon-win-x64.exe`.

| Folder | What it is |
|---|---|
| `core` | Reading Strata, connection tracking, alert rules and update checks, with no UI or OS code |
| `ui` | Drawing and input. It turns state and events into pixels with no window, so tests can drive it |
| `app` | The thin window shell, built on [winit](https://github.com/rust-windowing/winit) and [softbuffer](https://github.com/rust-windowing/softbuffer) |
| `tools/sign` | The release signing tool, which never ships |

Set `STRATAMON_SHOTS` to a folder when running the `ui` tests to save what each screen looks like.

The release profile sets `panic = "abort"`, so a panic closes the whole app. Code that handles anything
from the network must never `unwrap`.

### Making a release

For now a release is just a build: run `./scripts/publish.ps1`, which produces
`dist\StrataMon-win-x64.exe`, and hand that file out however you like.

The self-updater stays off until `REPOSITORY` and `PUBLIC_KEY` are set in `core/src/update.rs`, and
switching it on means hosting releases where the app can fetch them and signing every file with the
release key. Make the key once with `cargo run -p stratamon-sign -- keygen <folder>` (pick a folder
inside your own profile; the tool strips inherited access so only your account can read it), sign
each build with the same tool, and put the public key it prints into `PUBLIC_KEY`. Copies already
installed only trust that key, so losing it means they can't update themselves.

## License

MIT. See [LICENSE](LICENSE).

The libraries StrataMon uses are listed with their licenses in
[THIRD-PARTY.md](THIRD-PARTY.md). It bundles no fonts, and uses the ones on your system. Its tests
include a benchmark capture from [Strata](https://github.com/Niko1221/Strata), which is MIT licensed.
