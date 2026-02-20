mod error;
mod tui;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use conclave_lib::api::ApiClient;
use conclave_lib::config::{
    ClientConfig, SessionState, generate_initial_key_packages, load_group_mapping,
    save_group_mapping,
};
use conclave_lib::error::Error;
use conclave_lib::mls::MlsManager;
use conclave_lib::operations;

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

fn api_from_session(
    session: &SessionState,
    config: &ClientConfig,
) -> conclave_lib::error::Result<ApiClient> {
    let server_url = session
        .server_url
        .as_ref()
        .ok_or_else(|| Error::Other("not logged in — run login first".into()))?;
    let mut api = ApiClient::new(server_url, config.accept_invalid_certs);
    if let Some(token) = &session.token {
        api.set_token(token.clone());
    }
    Ok(api)
}

fn require_username(session: &SessionState) -> conclave_lib::error::Result<&str> {
    session
        .username
        .as_deref()
        .ok_or_else(|| Error::Other("not logged in — run login first".into()))
}

fn resolve_mls_group_id(data_dir: &Path, group: &str) -> conclave_lib::error::Result<String> {
    let mapping = load_group_mapping(data_dir);
    mapping
        .get(group)
        .cloned()
        .ok_or_else(|| Error::Other(format!("unknown group '{group}' — run join first")))
}

async fn run_command(cmd: Commands, config: &ClientConfig) -> conclave_lib::error::Result<()> {
    let mut session = SessionState::load(&config.data_dir);

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

            let mut login_api = ApiClient::new(&server, config.accept_invalid_certs);
            login_api.set_token(resp.token.clone());

            session.server_url = Some(server);
            session.token = Some(resp.token);
            session.user_id = Some(resp.user_id);
            session.username = Some(username.clone());
            session.save(&config.data_dir)?;

            std::fs::create_dir_all(&config.data_dir)?;
            let mls = MlsManager::new(&config.data_dir, &username)?;

            let entries = generate_initial_key_packages(&mls)?;
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
            let username = require_username(&session)?;
            let member_names: Vec<String> =
                members.split(',').map(|s| s.trim().to_string()).collect();

            let result =
                operations::create_group(&api, &name, member_names, &config.data_dir, username)
                    .await?;

            let mut mapping = load_group_mapping(&config.data_dir);
            mapping.insert(result.server_group_id.clone(), result.mls_group_id);
            save_group_mapping(&config.data_dir, &mapping);

            println!("Group '{name}' created (ID: {})", result.server_group_id);
        }

        Commands::Invite { group, members } => {
            let api = api_from_session(&session, config)?;
            let username = require_username(&session)?;
            let member_names: Vec<String> =
                members.split(',').map(|s| s.trim().to_string()).collect();

            let mls_group_id = resolve_mls_group_id(&config.data_dir, &group)?;

            let invited = operations::invite_members(
                &api,
                &group,
                &mls_group_id,
                member_names,
                &config.data_dir,
                username,
            )
            .await?;

            if invited.is_empty() {
                println!("No new members to invite.");
            } else {
                println!("Invited {} to group {group}", invited.join(", "));
            }
        }

        Commands::Groups => {
            let api = api_from_session(&session, config)?;
            let rooms = operations::load_rooms(&api).await?;
            if rooms.is_empty() {
                println!("No groups.");
            } else {
                for room in &rooms {
                    println!(
                        "  {} - {} [members: {}]",
                        room.group_id,
                        room.name,
                        room.members.join(", ")
                    );
                }
            }
        }

        Commands::Join => {
            let api = api_from_session(&session, config)?;
            let username = require_username(&session)?;

            let results = operations::accept_welcomes(&api, &config.data_dir, username).await?;

            if results.is_empty() {
                println!("No pending invitations.");
            } else {
                let mut mapping = load_group_mapping(&config.data_dir);
                for result in &results {
                    mapping.insert(result.group_id.clone(), result.mls_group_id.clone());
                    println!(
                        "Joined group '{}' (ID: {})",
                        result.group_name, result.group_id
                    );
                }
                save_group_mapping(&config.data_dir, &mapping);
            }
        }

        Commands::Send { group, message } => {
            let api = api_from_session(&session, config)?;
            let username = require_username(&session)?;
            let mls_group_id = resolve_mls_group_id(&config.data_dir, &group)?;

            let result = operations::send_message(
                &api,
                &group,
                &mls_group_id,
                &message,
                &config.data_dir,
                username,
            )
            .await?;
            println!("Message sent (seq: {})", result.sequence_num);
        }

        Commands::Messages { group } => {
            let api = api_from_session(&session, config)?;
            let username = require_username(&session)?;
            let mls_group_id = resolve_mls_group_id(&config.data_dir, &group)?;

            let fetched = operations::fetch_and_decrypt(
                &api,
                &group,
                0,
                &mls_group_id,
                &config.data_dir,
                username,
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
