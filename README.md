# MCTerminal
> [!CAUTION]
> This project is abandoned because of rejected AppID by Mojang. Online accounts don't work.

A Minecraft: Java Edition launcher for the terminal, written in Rust.

Every instance gets its own folder, so mods, worlds and settings never mix.
Libraries, assets and Java are shared, so a second instance of the same version costs almost nothing.

```zsh
mcterm auth --offline CoolBoy2002
mcterm install instance 1.21.11 --modloader fabric
mcterm install mod sodium -i 1.21.11-fabric
mcterm launch instance 1.21.11-fabric
```

## Features

- **Versions:** release, snapshot, beta and the very first `rd-...` versions (`mcterm list versions --snapshots --beta --pre-alpha`)
- **Modloaders:** Fabric, Quilt, NeoForge and Forge (`--modloader`, optionally `--loader-version`)
- **Java:** downloads the right runtime from Mojang, no system Java needed
- **Mods:** `install mod` from Modrinth (with required dependencies), `list mods`, `delete mod` by name or id read from the jar
- **Modpacks:** `install modpack pack.mrpack` (Modrinth format)
- **Accounts:** offline accounts, and Microsoft sign-in with the device code flow (mcterm never sees your password)

## Commands

```zsh
mcterm auth --offline NAME | --online
mcterm auth switch NAME
mcterm auth list-accounts

mcterm list versions [--snapshots] [--beta] [--pre-alpha]
mcterm list instances
mcterm list mods [-i INSTANCE]

mcterm install instance VERSION [--modloader fabric|quilt|neoforge|forge] [--name NAME]
mcterm install modpack PATH
mcterm install mod NAME [-i INSTANCE]
mcterm search QUERY [-i INSTANCE]

mcterm launch instance NAME

mcterm delete instance NAME [-y]
mcterm delete mod NAME [-i INSTANCE]
mcterm delete resourcepack NAME [-i INSTANCE]
```

## Where things are

Everything lives in `~/.mcterm` (set `MCTERM_HOME` to move it). Your `.minecraft` is never touched.

```
~/.mcterm/
├── accounts.json     accounts (readable only by you)
├── instances/NAME/   instance.json, and game/ with mods, saves, resourcepacks, ...
├── versions/         Minecraft and modloader versions
├── libraries/        shared
├── assets/           shared
└── runtimes/         Java from Mojang
```

## Microsoft sign-in

`mcterm auth --online` uses Microsoft's device code flow with an Azure app registration of your own.
Mojang has to approve that app's client ID before Minecraft accepts it.

```zsh
mcterm config client-id <YOUR-AZURE-APP-CLIENT-ID>
mcterm auth --online
```

Offline accounts need none of this.

## Build

```zsh
cargo build --release
```

Needs a recent stable Rust.

## Systems

- **macOS** (Apple Silicon and Intel): the one it is developed and tested on.
- **Linux** (x86_64): supported in code (Mojang's Java for Linux, Linux libraries and natives), but **not yet tested on a real machine**. Linux on ARM works only with a Java you install yourself (`JAVA_HOME`), since Mojang ships none.
- **Windows**: not supported yet.
