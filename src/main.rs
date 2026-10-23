#[macro_use]
mod say;
mod accounts;
mod auth;
mod delete;
mod download;
mod install;
mod launch;
mod loader;
mod modpack;
mod modrinth;
mod mods;
mod mojang;
mod paths;
mod runtime;
mod version;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};

/// A terminal launcher for Minecraft.
#[derive(Parser)]
#[command(name = "mcterm", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Log in, or manage accounts
    Auth(AuthArgs),
    /// Show or set mcterm's settings
    Config {
        #[command(subcommand)]
        what: ConfigCommand,
    },
    /// Install an instance or a modpack
    Install {
        #[command(subcommand)]
        what: InstallCommand,
    },
    /// List things
    List {
        #[command(subcommand)]
        what: ListCommand,
    },
    /// Search Modrinth for mods that fit an instance
    Search {
        /// What to look for
        query: String,
        /// Which instance (not needed when there is only one)
        #[arg(short, long)]
        instance: Option<String>,
    },
    /// Delete an instance, a mod or a resource pack
    Delete {
        #[command(subcommand)]
        what: DeleteCommand,
    },
    /// Launch an instance
    Launch {
        #[command(subcommand)]
        what: LaunchCommand,
    },
}

// `mcterm auth --online`, `mcterm auth --offline NAME`, `mcterm auth switch NAME`, `mcterm auth list-accounts`
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
struct AuthArgs {
    /// Log in with a Microsoft account (opens the browser)
    #[arg(long, conflicts_with = "offline")]
    online: bool,

    /// Add an offline account with this name
    #[arg(long, value_name = "NAME")]
    offline: Option<String>,

    #[command(subcommand)]
    command: Option<AuthCommand>,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// The Azure app (client ID) used for `auth --online`
    ClientId { value: Option<String> },
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Make an account the active one
    Switch { name: String },
    /// Show all saved accounts
    ListAccounts,
}

#[derive(Subcommand)]
enum InstallCommand {
    /// Create an instance of a Minecraft version
    Instance {
        /// Minecraft version, like 1.21.11
        version: String,
        /// fabric, quilt, neoforge or forge (vanilla if left out)
        #[arg(long)]
        modloader: Option<String>,
        /// Which version of the modloader (the newest stable one if left out)
        #[arg(long)]
        loader_version: Option<String>,
        /// Name of the instance (defaults to the version, plus the modloader)
        #[arg(long)]
        name: Option<String>,
    },
    /// Install a mod from Modrinth, with the mods it needs
    Mod {
        /// The mod's name or Modrinth slug
        name: String,
        /// Which instance (not needed when there is only one)
        #[arg(short, long)]
        instance: Option<String>,
    },
    /// Install a modpack (.mrpack)
    Modpack {
        path: PathBuf,
        /// Name of the instance (defaults to the pack's own name)
        #[arg(long)]
        name: Option<String>,
        /// Use this modloader version instead of the one the pack asks for
        #[arg(long)]
        loader_version: Option<String>,
    },
}

#[derive(Subcommand)]
enum ListCommand {
    /// Show installed instances
    Instances,
    /// Show the mods of an instance, by the id and name written inside each jar
    Mods {
        /// Which instance (not needed when there is only one)
        #[arg(short, long)]
        instance: Option<String>,
    },
    /// Show Minecraft versions you can install
    Versions {
        /// Include snapshots
        #[arg(long)]
        snapshots: bool,
        /// Include old betas (b1.x)
        #[arg(long)]
        beta: bool,
        /// Include the oldest versions: alphas, infdev, classic, and the very first rd-... ones
        #[arg(long = "pre-alpha")]
        pre_alpha: bool,
    },
}

#[derive(Subcommand)]
enum DeleteCommand {
    /// Delete a whole instance, with its mods, worlds and settings
    Instance {
        name: String,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
    },
    /// Delete a mod (the file name, or a unique piece of it)
    Mod {
        name: String,
        /// Which instance (not needed when there is only one)
        #[arg(short, long)]
        instance: Option<String>,
    },
    /// Delete a resource pack
    Resourcepack {
        name: String,
        /// Which instance (not needed when there is only one)
        #[arg(short, long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum LaunchCommand {
    /// Start an instance by name
    Instance { name: String },
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("mcterm: error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Config { what: ConfigCommand::ClientId { value } } => auth::config_client_id(value.as_deref())?,
        Command::Auth(auth) => match (auth.command, auth.online, auth.offline) {
            (Some(AuthCommand::Switch { name }), _, _) => accounts::switch(&name)?,
            (Some(AuthCommand::ListAccounts), _, _) => accounts::list()?,
            (None, true, _) => auth::login_online()?,
            (None, false, Some(name)) => accounts::add_offline(&name)?,
            (None, false, None) => bail!("give --online, --offline NAME, or a subcommand (see `mcterm auth --help`)"),
        },
        Command::Install { what } => match what {
            InstallCommand::Instance { version, modloader, loader_version, name } => {
                install::install_instance(&version, modloader.as_deref(), loader_version.as_deref(), name.as_deref())?
            }
            InstallCommand::Mod { name, instance } => modrinth::install_mod(&name, instance.as_deref())?,
            InstallCommand::Modpack { path, name, loader_version } => {
                modpack::install_modpack(&path, name.as_deref(), loader_version.as_deref())?
            }
        },
        Command::List { what: ListCommand::Mods { instance } } => mods::list_mods(instance.as_deref())?,
        Command::List { what: ListCommand::Instances } => install::list_instances()?,
        Command::List { what: ListCommand::Versions { snapshots, beta, pre_alpha } } => {
            let manifest = mojang::fetch_manifest()?;
            say!("latest release: {}   latest snapshot: {}", manifest.latest.release, manifest.latest.snapshot);
            for v in manifest.versions.iter().filter(|v| match v.kind.as_str() {
                "release" => true,
                "snapshot" => snapshots,
                "old_beta" => beta,
                "old_alpha" => pre_alpha,
                _ => false,
            }) {
                say!("{:<10} {}", v.kind, v.id);
            }
        }
        Command::Search { query, instance } => modrinth::search_mods(&query, instance.as_deref())?,
        Command::Delete { what } => match what {
            DeleteCommand::Instance { name, yes } => delete::delete_instance(&name, yes)?,
            DeleteCommand::Mod { name, instance } => delete::delete_mod(instance.as_deref(), &name)?,
            DeleteCommand::Resourcepack { name, instance } => delete::delete_resourcepack(instance.as_deref(), &name)?,
        },
        Command::Launch { what: LaunchCommand::Instance { name } } => launch::launch_instance(&name)?,
    }
    Ok(())
}
