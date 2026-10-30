# INSTALL.md — set up a fresh Mac, Windows or Linux machine for game-stack

**For agents (Claude Code etc.):** follow this top to bottom on the user's machine. Run each
command, check its **verify** line, and only then continue. Steps marked **👤 HUMAN** need the
person at the keyboard (an admin password, a reboot, a licence dialog, a firewall prompt).
Stop, tell them exactly what to click or type, and wait. Never type passwords yourself.

Assumes only: the operating system, a web browser, and Claude. Everything else is installed here.

## The fast path (any OS): let the doctor drive

1. Install **Git** and **Node.js ≥ 22** using step 1 + step 3 for your OS below (Linux: the first two blocks; macOS needs
   steps 1–3). That's all the doctor needs to run.
2. Get your own copy and run the doctor (no pnpm needed yet). With the GitHub CLI set up
   ([GitHub section](#github-identity-login-and-your-own-fork)), fork it into your account:
   ```sh
   gh repo fork bendgk/game-stack --clone && cd game-stack
   node scripts/setup.mjs --check
   ```
   (No GitHub account yet? `git clone https://github.com/bendgk/game-stack.git` works too. Fork later.)
3. It detects the OS and prints ✓/✗ for every prerequisite, with **the exact fix command for
   this OS** under each ✗. Apply the fixes (the per-OS sections below explain each one, and which
   need a human), open a new terminal, and re-run until every required line is ✓.
   Optional lines (•: Docker, Blender, Chrome) can wait.
4. `pnpm setup` → `pnpm verify` → `pnpm dev` ([details below](#every-os-get-the-game-running)).

If anything is unclear, follow the full step-by-step section for your OS.

## What gets installed

| Tool | Why | Version | Needed for |
| --- | --- | --- | --- |
| Git | version control: clone, commit, push | any recent | everything |
| GitHub CLI (`gh`) + a GitHub account | fork the starter into your own account, log in, push | latest | saving/sharing your game |
| C/C++ build tools (Xcode Command Line Tools / Visual Studio Build Tools) | Rust's linker | latest | everything |
| Node.js | scripts, web client, MCP servers | **≥ 22** (LTS 24 recommended) | everything |
| pnpm | JS package manager | **12.x** | everything |
| Rust (rustup) | server, game, wasm client | stable **≥ 1.88**; the repo's `rust-toolchain.toml` picks stable + the wasm target | everything |
| wasm-bindgen-cli | turns the Rust client into JS/TS bindings | **exactly 0.2.130** | everything |
| Docker Desktop (or OrbStack on Mac) | Dragonfly + Postgres, LAN-party image | latest | persistence + `pnpm stack` (optional for solo dev) |
| Blender | the `gs3d` 3D asset MCP | **5.x** | 3D art only (optional) |
| Google Chrome | play, and `pnpm e2e` | latest | testing (any modern browser can play) |

Total download: about 6 GB with Docker and Blender, about 2 GB without.

---

## macOS (Apple Silicon or Intel, macOS 13+)

Run these in **Terminal** (zsh).

### 1. Command Line Tools (compiler, linker, git)
```sh
xcode-select -p >/dev/null 2>&1 || xcode-select --install
```
**👤 HUMAN:** a dialog appears. Click *Install*, accept, and wait for it to finish (5–15 min).
**Verify:** `xcode-select -p` prints a path and `git --version` prints a version.

### 2. Homebrew (package manager)
```sh
command -v brew >/dev/null || /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```
**👤 HUMAN:** it asks for the Mac login password (typing shows nothing) and to press Return.
Then put brew on PATH (Apple Silicon path shown; Intel Macs use `/usr/local/bin/brew`):
```sh
echo 'eval "$(/opt/homebrew/bin/brew shellenv)"' >> ~/.zprofile
eval "$(/opt/homebrew/bin/brew shellenv)"
```
**Verify:** `brew --version`.

Then the GitHub CLI: `brew install gh`. **Verify:** `gh --version`.

### 3. Node.js + pnpm
```sh
brew install node@24
brew link --overwrite --force node@24
npm install -g pnpm@12
```
**Verify:** `node --version` → `v24.x` (≥ 22 is fine), `pnpm --version` → `12.x`.

### 4. Rust + wasm target + wasm-bindgen
```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
```
(The last line compiles for a few minutes.)
**Verify:** `rustc --version` (≥ 1.88), `wasm-bindgen --version` → `wasm-bindgen 0.2.130`.

### 5. Docker (pick one)
- **Docker Desktop:** `brew install --cask docker-desktop` (older Homebrew: `--cask docker`), then
  `open -a Docker`.
- **OrbStack** (lighter and faster on Mac; same `docker` commands): `brew install --cask orbstack`, then `open -a OrbStack`.

**👤 HUMAN:** on first launch, accept the licence/permissions dialog and wait until it says it's running.
**Verify:** `docker run --rm hello-world` prints "Hello from Docker!", and `docker compose version` works.

### 6. Blender (optional, for 3D art)
```sh
brew install --cask blender
```
**Verify:** `/Applications/Blender.app/Contents/MacOS/Blender --version` → `Blender 5.x`.

### 7. Chrome (optional, for `pnpm e2e`)
```sh
brew install --cask google-chrome
```

Then go to **[Get the game running](#every-os-get-the-game-running)**.

---

## Windows 10 (22H2+) / Windows 11

Run these in **PowerShell**. Steps marked *(admin)* need "Run as administrator"
(**👤 HUMAN:** right-click PowerShell → *Run as administrator*, and approve the UAC prompt).
`winget` ships with Windows. If it's missing, the human installs "App Installer" from the
Microsoft Store.

**After every `winget install`, refresh PATH in the current shell** (or open a new terminal):
```powershell
$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')
```

### 1. Git
```powershell
winget install --id Git.Git -e --accept-source-agreements --accept-package-agreements
```
**Verify:** `git --version`.

GitHub CLI:
```powershell
winget install --id GitHub.cli -e --accept-source-agreements --accept-package-agreements
```
**Verify:** `gh --version` (after refreshing PATH).

### 2. Visual Studio C++ Build Tools (Rust's linker), *(admin)*
```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```
(About 3–6 GB and 10–20 minutes. A progress window shows; let it finish.)
**Verify:** `Test-Path "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC"` → `True`.

### 3. Node.js + pnpm
```powershell
winget install --id OpenJS.NodeJS.LTS -e --accept-source-agreements --accept-package-agreements
# refresh PATH (see above), then:
npm install -g pnpm@12
```
**Verify:** `node --version` (≥ 22), `pnpm --version` → `12.x`.
If PowerShell blocks scripts ("running scripts is disabled"), run once:
`Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` (**👤 HUMAN** confirms with `Y`).

### 4. Rust + wasm target + wasm-bindgen
```powershell
winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
# refresh PATH, then:
rustup default stable-msvc
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
```
**Verify:** `rustc --version` (≥ 1.88; host `x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc`),
`wasm-bindgen --version` → `wasm-bindgen 0.2.130`.
If `cargo install` fails with `link.exe not found`, step 2 didn't finish. Re-run it.

### 5. WSL 2 + Docker Desktop, *(admin)*
```powershell
wsl --install --no-distribution
```
**👤 HUMAN:** reboot when asked, then continue in a new PowerShell.
```powershell
winget install --id Docker.DockerDesktop -e --accept-source-agreements --accept-package-agreements
Start-Process "$env:ProgramFiles\Docker\Docker\Docker Desktop.exe"
```
**👤 HUMAN:** accept the Docker licence, keep "Use WSL 2 based engine" on, and wait for
"Engine running". (Sign-in is optional; skip it.)
**Verify:** `docker run --rm hello-world` and `docker compose version`.

### 6. Blender (optional, for 3D art)
```powershell
winget install --id BlenderFoundation.Blender -e --accept-source-agreements --accept-package-agreements
```
**Verify:** `Get-ChildItem "$env:ProgramFiles\Blender Foundation" -Recurse -Filter blender.exe | Select-Object -First 1`
finds it. The `gs3d` MCP locates it automatically. Otherwise set
`$env:GS3D_BLENDER = "C:\path\to\blender.exe"`.

### 7. Chrome (optional, for `pnpm e2e`)
```powershell
winget install --id Google.Chrome -e --accept-source-agreements --accept-package-agreements
```

### 8. Firewall (so other devices on the LAN can join), *(admin)*
```powershell
New-NetFirewallRule -DisplayName "game-stack web (dev)" -Direction Inbound -Protocol TCP -LocalPort 5173 -Action Allow -Profile Private
New-NetFirewallRule -DisplayName "game-stack http" -Direction Inbound -Protocol TCP -LocalPort 8080 -Action Allow -Profile Private
New-NetFirewallRule -DisplayName "game-stack webrtc" -Direction Inbound -Protocol UDP -LocalPort 40000 -Action Allow -Profile Private
```
**👤 HUMAN:** make sure the Wi-Fi/Ethernet network is set to **Private** (Settings → Network →
your network → Network profile type → Private). On a Public network Windows blocks LAN games.
If Windows shows "Allow access?" for Node or gs-server later, click **Allow** on private networks.

---

## Linux (Ubuntu/Debian; Fedora equivalents in brackets)

```sh
sudo apt update && sudo apt install -y git build-essential pkg-config curl ca-certificates   # [sudo dnf install -y git gcc gcc-c++ make pkgconf curl]
```
**👤 HUMAN:** `sudo` asks for the password.

GitHub CLI: `sudo apt install -y gh` [`sudo dnf install -y gh`]. On older Ubuntu, follow
https://github.com/cli/cli/blob/trunk/docs/install_linux.md. **Verify:** `gh --version`.

Node.js 24 via fnm, then pnpm:
```sh
curl -fsSL https://fnm.vercel.app/install | bash
source ~/.bashrc            # or open a new terminal
fnm install 24 && fnm default 24
npm install -g pnpm@12
```
Rust + wasm:
```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
```
Docker Engine (optional): follow https://docs.docker.com/engine/install/ for your distro, then
`sudo usermod -aG docker $USER` (**👤 HUMAN:** log out and back in). Blender (optional):
`sudo snap install blender --classic`. Chrome (optional): from google.com/chrome.
Firewall (if `ufw` is active): `sudo ufw allow 5173/tcp && sudo ufw allow 8080/tcp && sudo ufw allow 40000/udp`.
**Verify** with `node scripts/setup.mjs --check`.

---

## GitHub: identity, login and your own fork

Do this once per machine, on any OS (same commands in Terminal / PowerShell).

1. **👤 HUMAN:** have a GitHub account (free at https://github.com/signup).
2. Tell git who you are (shows on your commits; use the email of your GitHub account):
   ```sh
   git config --global user.name "Your Name"
   git config --global user.email "you@example.com"
   git config --global init.defaultBranch main
   git config --global core.autocrlf false     # keep LF line endings (the repo's checks expect them)
   ```
   **Verify:** `git config --global user.email` prints it. (Agents: ask the person for their name
   and email. Don't invent them.)
3. Log in to GitHub:
   ```sh
   gh auth login --hostname github.com --git-protocol https --web
   gh auth setup-git
   ```
   **👤 HUMAN:** it prints a one-time code and opens the browser. Paste the code and approve.
   **Verify:** `gh auth status` shows "Logged in to github.com".
4. Fork the starter into your account and clone your fork:
   ```sh
   gh repo fork bendgk/game-stack --clone --remote
   cd game-stack
   ```
   `origin` is now your fork (push here) and `upstream` is the starter (pull updates from here).
5. Save and share your work:
   ```sh
   git add -A && git commit -m "My first change"
   git push
   ```
   Get starter updates later: `git pull upstream main`.

---

## Every OS: get the game running

### 1. Get the code
Your fork from the [GitHub section](#github-identity-login-and-your-own-fork)
(`gh repo fork bendgk/game-stack --clone --remote`), or a plain
`git clone https://github.com/bendgk/game-stack.git`.

### 2. One-time project setup
```sh
pnpm doctor     # check only (same as node scripts/setup.mjs --check)
pnpm setup      # check + install the wasm target / wasm-bindgen if missing + pnpm install + first wasm build
```
**Verify:** it ends with `Ready.`

### 3. Prove everything works
```sh
pnpm verify
```
Formatting, lints, all Rust tests (including the netcode soak tests), the wire contract, the wasm
build, TypeScript, the MCP servers' tests, and the native-vs-wasm determinism check. The first
run compiles everything (5–15 min).
**Verify:** it ends with `✓ verify passed`.

### 4. Run it
```sh
pnpm dev
```
Starts Dragonfly + Postgres (Docker), the Rust server and Vite, with rebuild-on-save.
**Verify:** the log shows `open http://<LAN-IP>:8080` from the server and Vite's
`Network: http://<LAN-IP>:5173/`. Open `http://localhost:5173`, enter a name, press
**Play on server**. You should be in the arena. Open a second tab with the same room to see multiplayer.
Without Docker running, it still works (in-memory store, no leaderboard).

### 5. Optional checks
```sh
pnpm e2e                                                 # drives Chrome against pnpm dev (needs Chrome)
pnpm netbot -- --room test --clients 4 --seconds 10      # headless WebRTC players against pnpm dev
GS3D_IT=1 node --test tools/mcp-3d/lib.test.mjs          # real-Blender tests (macOS/Linux shell)
```
(PowerShell form of the last one: `$env:GS3D_IT=1; node --test tools/mcp-3d/lib.test.mjs`.)

### 6. MCP servers (asset studios)
Nothing to install: `.mcp.json` in the repo registers `gs2d` (needs only Node) and `gs3d` (needs
Blender). **👤 HUMAN:** when Claude Code first opens the repo it asks to trust/enable the
project's MCP servers. Approve them. **Verify:** in Claude Code, `/mcp` lists `gs2d` and `gs3d`
as connected (`gs3d` starts Blender on its first tool call).

### 7. LAN party (optional)
```sh
pnpm stack        # builds the Docker image and serves everything on :8080
```
Players on the same Wi-Fi open the printed `http://<LAN-IP>:8080`. Allow TCP 8080 and UDP 40000
through the firewall (Windows: step 8 above; macOS: click **Allow** if asked about incoming connections).

---

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `git commit` says "Please tell me who you are" | Set `user.name` / `user.email` (GitHub section, step 2). |
| `git push` asks for a password / is denied | `gh auth login` then `gh auth setup-git`; make sure `origin` is YOUR fork (`git remote -v`). |
| Not sure what's missing | `node scripts/setup.mjs --check` (the doctor) lists every prerequisite with the fix for this OS. |
| `command not found` / `not recognized` right after installing | New shell, or refresh PATH (Windows snippet above; macOS: `. "$HOME/.cargo/env"` and `eval "$(brew shellenv)"`). |
| `link.exe` / `cc` not found while compiling Rust | Build tools missing: Windows step 2 / macOS step 1. |
| `wasm-bindgen` version mismatch | `cargo install wasm-bindgen-cli --version 0.2.130 --locked --force`. |
| `pnpm` refuses to run scripts on Windows | `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. |
| `docker: Cannot connect to the Docker daemon` | Start Docker Desktop / OrbStack and wait for "running". |
| WebRTC connect timed out (other devices) | Firewall (UDP 40000), network set to Private, same Wi-Fi (guest networks often isolate devices). |
| `pnpm verify` contract check fails on a fresh Windows clone | The clone has CRLF line endings: `git config core.autocrlf false`, then `git rm --cached -r . && git reset --hard`. |
| Rust edition/let-chain errors | Rust too old: `rustup update stable` (needs ≥ 1.88). |
| `gs3d` tools say "cannot start Blender" | Install Blender 5.x or set `GS3D_BLENDER` to its executable path. |
| Ports 5173/8080/40000/5432/6379 already in use | Another `pnpm dev`/`pnpm stack` or a local Postgres/Redis is running; stop it (`pnpm stack down`). |

When everything above passes, read [`README.md`](README.md) (onboarding) and
[`AGENTS.md`](AGENTS.md) (how agents work in this repo).
