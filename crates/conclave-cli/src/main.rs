mod error;
mod notification;
mod tui;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde::Deserialize;

use conclave_client::api::ApiClient;
use conclave_client::config::{ClientConfig, SessionState, load_group_mapping, save_group_mapping};
use conclave_client::error::Error;
use conclave_client::operations::{self, AuthResult};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMethod {
    #[default]
    Native,
    Bell,
    Both,
    None,
}

#[derive(Deserialize)]
struct CliConfig {
    #[serde(flatten)]
    client: ClientConfig,
    #[serde(default)]
    notifications: NotificationMethod,
}

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
    /// Register a new account and login.
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
    },
    /// Invite a user to an existing group.
    Invite {
        #[arg(short, long)]
        group: i64,
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
        group: i64,
        #[arg(short, long)]
        message: String,
    },
    /// Fetch and decrypt messages from a group.
    Messages {
        #[arg(short, long)]
        group: i64,
    },
    /// Set your display name (alias).
    Nick {
        #[arg(short, long)]
        alias: String,
    },
    /// Set a group's display alias.
    Topic {
        #[arg(short, long)]
        group: i64,
        #[arg(short, long)]
        alias: String,
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

    let (mut config, notifications) = if let Some(config_path) = &cli.config {
        let contents = std::fs::read_to_string(config_path)?;
        let cli_config: CliConfig = toml::from_str(&contents)?;
        (cli_config.client, cli_config.notifications)
    } else {
        (ClientConfig::load(), NotificationMethod::default())
    };

    // CLI --data-dir overrides config file and env var.
    if let Some(data_dir) = cli.data_dir {
        config.data_dir = data_dir;
    }

    match cli.command {
        None => {
            // Interactive TUI mode.
            tui::run(&config, &notifications).await?;
        }
        Some(cmd) => {
            run_command(cmd, &config).await?;
        }
    }

    Ok(())
}

fn api_from_session(
    session: &SessionState,
    config: &ClientConfig,
) -> conclave_client::error::Result<ApiClient> {
    let server_url = session
        .server_url
        .as_ref()
        .ok_or_else(|| Error::Other("not logged in -- run login first".into()))?;
    let mut api = ApiClient::new(server_url, config.accept_invalid_certs);
    if let Some(token) = &session.token {
        api.set_token(token.clone());
    }
    Ok(api)
}

fn require_user_id(session: &SessionState) -> conclave_client::error::Result<i64> {
    session
        .user_id
        .ok_or_else(|| Error::Other("not logged in -- run login first".into()))
}

fn resolve_mls_group_id(data_dir: &Path, group_id: i64) -> conclave_client::error::Result<String> {
    let mapping = load_group_mapping(data_dir);
    mapping
        .get(&group_id)
        .cloned()
        .ok_or_else(|| Error::Other(format!("unknown group '{group_id}' -- run join first")))
}

fn print_auth_result(action: &str, result: &AuthResult) {
    println!(
        "{action} as {} (user ID {})",
        result.username, result.user_id
    );
    let count = result.key_packages_uploaded;
    println!(
        "{count} key packages uploaded (1 last-resort + {} regular).",
        count - 1
    );
}

async fn run_command(cmd: Commands, config: &ClientConfig) -> conclave_client::error::Result<()> {
    let session = SessionState::load(&config.data_dir);

    match cmd {
        Commands::Register {
            server,
            username,
            password,
        } => {
            let result = operations::register_and_login(
                &server,
                &username,
                &password,
                config.accept_invalid_certs,
                &config.data_dir,
            )
            .await?;
            result.save_session(&config.data_dir)?;
            print_auth_result("Registered and logged in", &result);
        }

        Commands::Login {
            server,
            username,
            password,
        } => {
            let result = operations::login(
                &server,
                &username,
                &password,
                config.accept_invalid_certs,
                &config.data_dir,
            )
            .await?;
            result.save_session(&config.data_dir)?;
            print_auth_result("Logged in", &result);
        }

        Commands::CreateGroup { name } => {
            let api = api_from_session(&session, config)?;
            let user_id = require_user_id(&session)?;

            let result =
                operations::create_group(&api, None, &name, &config.data_dir, user_id).await?;

            let mut mapping = load_group_mapping(&config.data_dir);
            mapping.insert(result.server_group_id, result.mls_group_id);
            save_group_mapping(&config.data_dir, &mapping);

            println!("Group '{name}' created (ID: {})", result.server_group_id);
        }

        Commands::Invite { group, members } => {
            let api = api_from_session(&session, config)?;
            let user_id = require_user_id(&session)?;
            let member_names: Vec<String> = members
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if member_names.is_empty() {
                return Err(Error::Other("no valid member names provided".into()));
            }

            // Resolve usernames to user IDs at the UI boundary.
            let mut member_ids = Vec::new();
            for username in &member_names {
                let user_info = api.get_user_by_username(username).await?;
                member_ids.push(user_info.user_id);
            }

            let mls_group_id = resolve_mls_group_id(&config.data_dir, group)?;

            let invited = operations::invite_members(
                &api,
                group,
                &mls_group_id,
                member_ids.clone(),
                &config.data_dir,
                user_id,
            )
            .await?;

            if invited.is_empty() {
                println!("No new members to invite.");
            } else {
                let invited_names: Vec<&str> = member_names
                    .iter()
                    .zip(member_ids.iter())
                    .filter(|(_, mid)| invited.contains(mid))
                    .map(|(name, _)| name.as_str())
                    .collect();
                println!("Invited {} to group {group}", invited_names.join(", "));
            }
        }

        Commands::Groups => {
            let api = api_from_session(&session, config)?;
            let rooms = operations::load_rooms(&api).await?;
            if rooms.is_empty() {
                println!("No groups.");
            } else {
                for room in &rooms {
                    let member_display: Vec<&str> =
                        room.members.iter().map(|m| m.display_name()).collect();
                    println!(
                        "  {} - {} [members: {}]",
                        room.group_id,
                        room.display_name(),
                        member_display.join(", ")
                    );
                }
            }
        }

        Commands::Join => {
            let api = api_from_session(&session, config)?;
            let user_id = require_user_id(&session)?;

            let results = operations::accept_welcomes(&api, &config.data_dir, user_id).await?;

            if results.is_empty() {
                println!("No pending invitations.");
            } else {
                let mut mapping = load_group_mapping(&config.data_dir);
                for result in &results {
                    mapping.insert(result.group_id, result.mls_group_id.clone());
                    let id_string = result.group_id.to_string();
                    let display = result.group_alias.as_deref().unwrap_or(&id_string);
                    println!("Joined group '{}' (ID: {})", display, result.group_id);
                }
                save_group_mapping(&config.data_dir, &mapping);
            }
        }

        Commands::Send { group, message } => {
            let api = api_from_session(&session, config)?;
            let user_id = require_user_id(&session)?;
            let mls_group_id = resolve_mls_group_id(&config.data_dir, group)?;

            let result = operations::send_message(
                &api,
                group,
                &mls_group_id,
                &message,
                &config.data_dir,
                user_id,
            )
            .await?;
            println!("Message sent (seq: {})", result.sequence_num);
        }

        Commands::Nick { alias } => {
            let api = api_from_session(&session, config)?;
            api.update_profile(&alias).await?;
            println!("Alias set to: {alias}");
        }

        Commands::Topic { group, alias } => {
            let api = api_from_session(&session, config)?;
            api.update_group(group, Some(&alias)).await?;
            println!("Room alias set to: {alias}");
        }

        Commands::Messages { group } => {
            let api = api_from_session(&session, config)?;
            let user_id = require_user_id(&session)?;
            let mls_group_id = resolve_mls_group_id(&config.data_dir, group)?;

            let rooms = operations::load_rooms(&api).await?;
            let members: Vec<conclave_client::state::RoomMember> = rooms
                .iter()
                .find(|r| r.group_id == group)
                .map(|r| r.members.iter().map(|m| m.to_room_member()).collect())
                .unwrap_or_default();

            let fetched = operations::fetch_and_decrypt(
                &api,
                group,
                0,
                &mls_group_id,
                &config.data_dir,
                user_id,
                &members,
            )
            .await?;

            if fetched.messages.is_empty() {
                println!("No messages.");
            } else {
                for msg in &fetched.messages {
                    if msg.is_system {
                        println!("  [{}] * {}", msg.sequence_num, msg.content);
                    } else {
                        println!("  [{}] {}: {}", msg.sequence_num, msg.sender, msg.content);
                    }
                }
            }
        }
    }

    Ok(())
}
