mod api;
mod config;
mod error;
mod mls;
mod tui;

use std::collections::HashMap;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::api::ApiClient;
use crate::config::{ClientConfig, SessionState};
use crate::error::{Error, Result};
use crate::mls::MlsManager;

#[derive(Parser)]
#[command(name = "conclave-client", about = "Conclave E2EE messaging client")]
struct Cli {
    /// Path to the client configuration file.
    #[arg(short, long, default_value = "conclave-client.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register a new account on the server.
    Register {
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        password: String,
    },
    /// Login to the server.
    Login {
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        password: String,
    },
    /// Generate and upload a new MLS key package.
    Keygen,
    /// Create a new encrypted group.
    CreateGroup {
        #[arg(short, long)]
        name: String,
        /// Comma-separated list of member usernames to invite.
        #[arg(short, long)]
        members: String,
    },
    /// Invite a user to an existing group.
    Invite {
        #[arg(short, long)]
        group: String,
        /// Comma-separated list of usernames to invite.
        #[arg(short, long)]
        members: String,
    },
    /// List groups you are a member of.
    Groups,
    /// Accept pending group invitations.
    Join,
    /// Send an encrypted message to a group.
    Send {
        #[arg(short, long)]
        group: String,
        #[arg(short, long)]
        message: String,
    },
    /// Fetch and decrypt messages from a group.
    Messages {
        #[arg(short, long)]
        group: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "conclave_client=info".into()),
        )
        .init();

    let cli = Cli::parse();

    let config: ClientConfig = if cli.config.exists() {
        let contents = std::fs::read_to_string(&cli.config)?;
        toml::from_str(&contents)?
    } else {
        ClientConfig::default()
    };

    match cli.command {
        None => {
            // Interactive TUI mode.
            tui::run(&config).await?;
        }
        Some(cmd) => {
            run_command(cmd, &config).await?;
        }
    }

    Ok(())
}

async fn run_command(cmd: Commands, config: &ClientConfig) -> Result<()> {
    let mut api = ApiClient::new(&config.server_url);
    let mut session = SessionState::load(&config.data_dir);

    if let Some(token) = &session.token {
        api.set_token(token.clone());
    }

    match cmd {
        Commands::Register { username, password } => {
            let resp = api.register(&username, &password).await?;
            println!("Registered as user ID {}", resp.user_id);
        }

        Commands::Login { username, password } => {
            let resp = api.login(&username, &password).await?;

            api.set_token(resp.token.clone());
            session.token = Some(resp.token);
            session.user_id = Some(resp.user_id);
            session.username = Some(username.clone());
            session.save(&config.data_dir)?;

            // Initialize MLS identity.
            std::fs::create_dir_all(&config.data_dir)?;
            let _mls = MlsManager::new(&config.data_dir, &username)?;

            println!("Logged in as {username} (user ID {})", resp.user_id);
        }

        Commands::Keygen => {
            let username = session
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in — run login first".into()))?;
            let mls = MlsManager::new(&config.data_dir, username)?;
            let kp = mls.generate_key_package()?;
            api.upload_key_package(kp).await?;
            println!("Key package generated and uploaded.");
        }

        Commands::CreateGroup { name, members } => {
            let member_names: Vec<String> =
                members.split(',').map(|s| s.trim().to_string()).collect();

            let resp = api.create_group(&name, member_names).await?;
            let server_group_id = resp.group_id.clone();

            let username = session
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let (mls_group_id, commit_bytes, welcome_map, group_info_bytes) =
                mls.create_group(&resp.member_key_packages)?;

            api.upload_commit(
                &server_group_id,
                commit_bytes,
                welcome_map,
                group_info_bytes,
            )
            .await?;

            // Save group mapping.
            let udir = user_data_dir(&config.data_dir, username);
            let mut mapping = load_group_mapping(&udir);
            mapping.insert(server_group_id.clone(), mls_group_id);
            save_group_mapping(&udir, &mapping);

            println!("Group '{name}' created (ID: {server_group_id})");
        }

        Commands::Invite { group, members } => {
            let member_names: Vec<String> =
                members.split(',').map(|s| s.trim().to_string()).collect();

            let resp = api.invite_to_group(&group, member_names).await?;

            if resp.member_key_packages.is_empty() {
                println!("No new members to invite.");
                return Ok(());
            }

            let username = session
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let udir = user_data_dir(&config.data_dir, username);
            let mapping = load_group_mapping(&udir);
            let mls_group_id = mapping
                .get(&group)
                .ok_or_else(|| Error::Other(format!("unknown group '{group}' — run join first")))?;

            let mls = MlsManager::new(&config.data_dir, username)?;

            let (commit_bytes, welcome_map, group_info_bytes) =
                mls.invite_to_group(mls_group_id, &resp.member_key_packages)?;

            api.upload_commit(&group, commit_bytes, welcome_map, group_info_bytes)
                .await?;

            let invited: Vec<String> = resp.member_key_packages.keys().cloned().collect();
            println!("Invited {} to group {group}", invited.join(", "));
        }

        Commands::Groups => {
            let resp = api.list_groups().await?;
            if resp.groups.is_empty() {
                println!("No groups.");
            } else {
                for g in &resp.groups {
                    let members: Vec<&str> =
                        g.members.iter().map(|m| m.username.as_str()).collect();
                    println!(
                        "  {} - {} [members: {}]",
                        g.group_id,
                        g.name,
                        members.join(", ")
                    );
                }
            }
        }

        Commands::Join => {
            let resp = api.list_pending_welcomes().await?;
            if resp.welcomes.is_empty() {
                println!("No pending invitations.");
                return Ok(());
            }

            let username = session
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let udir = user_data_dir(&config.data_dir, username);
            let mut mapping = load_group_mapping(&udir);

            for welcome in &resp.welcomes {
                let mls_group_id = mls.join_group(&welcome.welcome_message)?;
                mapping.insert(welcome.group_id.clone(), mls_group_id);
                println!(
                    "Joined group '{}' (ID: {})",
                    welcome.group_name, welcome.group_id
                );
            }

            save_group_mapping(&udir, &mapping);
        }

        Commands::Send { group, message } => {
            let username = session
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let udir = user_data_dir(&config.data_dir, username);
            let mapping = load_group_mapping(&udir);
            let mls_group_id = mapping
                .get(&group)
                .ok_or_else(|| Error::Other(format!("unknown group '{group}' — run join first")))?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let encrypted = mls.encrypt_message(mls_group_id, message.as_bytes())?;

            let resp = api.send_message(&group, encrypted).await?;
            println!("Message sent (seq: {})", resp.sequence_num);
        }

        Commands::Messages { group } => {
            let username = session
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let udir = user_data_dir(&config.data_dir, username);
            let mapping = load_group_mapping(&udir);
            let mls_group_id = mapping
                .get(&group)
                .ok_or_else(|| Error::Other(format!("unknown group '{group}' — run join first")))?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let resp = api.get_messages(&group, 0).await?;

            if resp.messages.is_empty() {
                println!("No messages.");
            } else {
                for msg in &resp.messages {
                    match mls.decrypt_message(mls_group_id, &msg.mls_message) {
                        Ok(Some(plaintext)) => {
                            let text = String::from_utf8_lossy(&plaintext);
                            println!("  [{}] {}: {}", msg.sequence_num, msg.sender_username, text);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            eprintln!(
                                "  [{}] {}: <decryption failed: {}>",
                                msg.sequence_num, msg.sender_username, e
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Compute the per-user data directory for MLS state and group mappings.
fn user_data_dir(data_dir: &std::path::Path, username: &str) -> std::path::PathBuf {
    data_dir.join("users").join(username)
}

fn load_group_mapping(user_dir: &std::path::Path) -> HashMap<String, String> {
    let path = user_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

fn save_group_mapping(user_dir: &std::path::Path, mapping: &HashMap<String, String>) {
    let path = user_dir.join("group_mapping.toml");
    if let Ok(contents) = toml::to_string_pretty(mapping) {
        let _ = std::fs::write(path, contents);
    }
}
