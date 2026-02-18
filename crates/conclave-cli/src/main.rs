mod error;
mod tui;

use std::collections::HashMap;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use conclave_lib::api::ApiClient;
use conclave_lib::config::{ClientConfig, SessionState};
use conclave_lib::mls::MlsManager;

#[derive(Parser)]
#[command(name = "conclave-cli", about = "Conclave E2EE messaging client")]
struct Cli {
    /// Path to the client configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Override the data directory (default: $CONCLAVE_DATA_DIR or XDG data dir).
    #[arg(short, long)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register a new account on the server.
    Register {
        #[arg(short, long)]
        server: String,
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        password: String,
    },
    /// Login to the server.
    Login {
        #[arg(short, long)]
        server: String,
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        password: String,
    },
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
                .unwrap_or_else(|_| "conclave_cli=info".into()),
        )
        .init();

    let cli = Cli::parse();

    let mut config: ClientConfig = if let Some(config_path) = &cli.config {
        let contents = std::fs::read_to_string(config_path)?;
        toml::from_str(&contents)?
    } else {
        ClientConfig::load()
    };

    // CLI --data-dir overrides config file and env var.
    if let Some(data_dir) = cli.data_dir {
        config.data_dir = data_dir;
    }

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

async fn run_command(cmd: Commands, config: &ClientConfig) -> conclave_lib::error::Result<()> {
    let mut session = SessionState::load(&config.data_dir);

    /// Create an authenticated ApiClient from the saved session.
    fn api_from_session(
        session: &SessionState,
        config: &ClientConfig,
    ) -> conclave_lib::error::Result<ApiClient> {
        let server_url = session.server_url.as_ref().ok_or_else(|| {
            conclave_lib::error::Error::Other("not logged in — run login first".into())
        })?;
        let mut api = ApiClient::new(server_url, config.accept_invalid_certs);
        if let Some(token) = &session.token {
            api.set_token(token.clone());
        }
        Ok(api)
    }

    match cmd {
        Commands::Register {
            server,
            username,
            password,
        } => {
            let api = ApiClient::new(&server, config.accept_invalid_certs);
            let resp = api.register(&username, &password).await?;
            println!("Registered as user ID {}", resp.user_id);
        }

        Commands::Login {
            server,
            username,
            password,
        } => {
            let api = ApiClient::new(&server, config.accept_invalid_certs);
            let resp = api.login(&username, &password).await?;

            // Set up an authenticated API client before moving values into session.
            let mut login_api = ApiClient::new(&server, config.accept_invalid_certs);
            login_api.set_token(resp.token.clone());

            session.server_url = Some(server);
            session.token = Some(resp.token);
            session.user_id = Some(resp.user_id);
            session.username = Some(username.clone());
            session.save(&config.data_dir)?;

            // Initialize MLS identity.
            std::fs::create_dir_all(&config.data_dir)?;
            let mls = MlsManager::new(&config.data_dir, &username)?;

            // Auto-generate and upload key packages (1 last-resort + 5 regular).
            let mut entries = Vec::with_capacity(6);
            let last_resort = mls.generate_last_resort_key_package()?;
            entries.push((last_resort, true));
            for kp in mls.generate_key_packages(5)? {
                entries.push((kp, false));
            }
            let count = entries.len();
            login_api.upload_key_packages(entries).await?;

            println!("Logged in as {username} (user ID {})", resp.user_id);
            println!(
                "{count} key packages uploaded (1 last-resort + {} regular).",
                count - 1
            );
        }

        Commands::CreateGroup { name, members } => {
            let api = api_from_session(&session, config)?;
            let member_names: Vec<String> =
                members.split(',').map(|s| s.trim().to_string()).collect();

            let resp = api.create_group(&name, member_names).await?;
            let server_group_id = resp.group_id.clone();

            let username = session
                .username
                .as_ref()
                .ok_or_else(|| conclave_lib::error::Error::Other("not logged in".into()))?;
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
            let mut mapping = load_group_mapping(&config.data_dir);
            mapping.insert(server_group_id.clone(), mls_group_id);
            save_group_mapping(&config.data_dir, &mapping);

            println!("Group '{name}' created (ID: {server_group_id})");
        }

        Commands::Invite { group, members } => {
            let api = api_from_session(&session, config)?;
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
                .ok_or_else(|| conclave_lib::error::Error::Other("not logged in".into()))?;

            let mapping = load_group_mapping(&config.data_dir);
            let mls_group_id = mapping.get(&group).ok_or_else(|| {
                conclave_lib::error::Error::Other(format!(
                    "unknown group '{group}' — run join first"
                ))
            })?;

            let mls = MlsManager::new(&config.data_dir, username)?;

            let (commit_bytes, welcome_map, group_info_bytes) =
                mls.invite_to_group(mls_group_id, &resp.member_key_packages)?;

            api.upload_commit(&group, commit_bytes, welcome_map, group_info_bytes)
                .await?;

            let invited: Vec<String> = resp.member_key_packages.keys().cloned().collect();
            println!("Invited {} to group {group}", invited.join(", "));
        }

        Commands::Groups => {
            let api = api_from_session(&session, config)?;
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
            let api = api_from_session(&session, config)?;
            let resp = api.list_pending_welcomes().await?;
            if resp.welcomes.is_empty() {
                println!("No pending invitations.");
                return Ok(());
            }

            let username = session
                .username
                .as_ref()
                .ok_or_else(|| conclave_lib::error::Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let mut mapping = load_group_mapping(&config.data_dir);

            for welcome in &resp.welcomes {
                let mls_group_id = mls.join_group(&welcome.welcome_message)?;

                // Delete the welcome from the server so it is not re-processed.
                let _ = api.accept_welcome(welcome.welcome_id).await;

                mapping.insert(welcome.group_id.clone(), mls_group_id);
                println!(
                    "Joined group '{}' (ID: {})",
                    welcome.group_name, welcome.group_id
                );
            }

            save_group_mapping(&config.data_dir, &mapping);

            // Key packages are single-use (RFC 9420 §10); upload a fresh
            // replacement so we remain available for future group invitations.
            let kp = mls.generate_key_package()?;
            api.upload_key_packages(vec![(kp, false)]).await?;
        }

        Commands::Send { group, message } => {
            let api = api_from_session(&session, config)?;
            let username = session
                .username
                .as_ref()
                .ok_or_else(|| conclave_lib::error::Error::Other("not logged in".into()))?;

            let mapping = load_group_mapping(&config.data_dir);
            let mls_group_id = mapping.get(&group).ok_or_else(|| {
                conclave_lib::error::Error::Other(format!(
                    "unknown group '{group}' — run join first"
                ))
            })?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let encrypted = mls.encrypt_message(mls_group_id, message.as_bytes())?;

            let resp = api.send_message(&group, encrypted).await?;
            println!("Message sent (seq: {})", resp.sequence_num);
        }

        Commands::Messages { group } => {
            let api = api_from_session(&session, config)?;
            let username = session
                .username
                .as_ref()
                .ok_or_else(|| conclave_lib::error::Error::Other("not logged in".into()))?;

            let mapping = load_group_mapping(&config.data_dir);
            let mls_group_id = mapping.get(&group).ok_or_else(|| {
                conclave_lib::error::Error::Other(format!(
                    "unknown group '{group}' — run join first"
                ))
            })?;
            let mls = MlsManager::new(&config.data_dir, username)?;

            let resp = api.get_messages(&group, 0).await?;

            if resp.messages.is_empty() {
                println!("No messages.");
            } else {
                for msg in &resp.messages {
                    match mls.decrypt_message(mls_group_id, &msg.mls_message) {
                        Ok(conclave_lib::mls::DecryptedMessage::Application(plaintext)) => {
                            let text = String::from_utf8_lossy(&plaintext);
                            println!("  [{}] {}: {}", msg.sequence_num, msg.sender_username, text);
                        }
                        Ok(conclave_lib::mls::DecryptedMessage::Commit(info)) => {
                            for added in &info.members_added {
                                println!("  [{}] * {added} joined", msg.sequence_num);
                            }
                            for removed in &info.members_removed {
                                println!("  [{}] * {removed} was removed", msg.sequence_num);
                            }
                        }
                        Ok(conclave_lib::mls::DecryptedMessage::Failed(reason)) => {
                            eprintln!(
                                "  [{}] {}: <decryption failed: {reason}>",
                                msg.sequence_num, msg.sender_username
                            );
                        }
                        Ok(conclave_lib::mls::DecryptedMessage::None) => {}
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

fn load_group_mapping(data_dir: &std::path::Path) -> HashMap<String, String> {
    let path = data_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

fn save_group_mapping(data_dir: &std::path::Path, mapping: &HashMap<String, String>) {
    let path = data_dir.join("group_mapping.toml");
    if let Ok(contents) = toml::to_string_pretty(mapping) {
        let _ = std::fs::write(path, contents);
    }
}
