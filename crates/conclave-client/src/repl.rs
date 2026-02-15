use std::collections::HashMap;
use std::sync::Arc;

use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use tokio::sync::Mutex;

use crate::api::ApiClient;
use crate::config::{ClientConfig, SessionState};
use crate::error::{Error, Result};
use crate::mls::MlsManager;

/// Maps server-side group UUID → MLS group ID (hex of MLS internal bytes).
type GroupMapping = HashMap<String, String>;

pub async fn run(config: &ClientConfig) -> Result<()> {
    let session = SessionState::load(&config.data_dir);
    let api = Arc::new(Mutex::new(ApiClient::new(&config.server_url)));

    if let Some(token) = &session.token {
        api.lock().await.set_token(token.clone());
    }

    let session = Arc::new(Mutex::new(session));
    let config_data_dir = config.data_dir.clone();

    // Load group mapping (server group UUID -> MLS group ID).
    // Initially empty; will be loaded from the per-user directory on login.
    let group_mapping = Arc::new(Mutex::new({
        let sess = session.lock().await;
        if let Some(username) = &sess.username {
            load_group_mapping(&config.data_dir.join("users").join(username))
        } else {
            HashMap::new()
        }
    }));

    let mut rl = DefaultEditor::new().map_err(|e| Error::Other(e.to_string()))?;

    println!("Conclave interactive client. Type /help for commands.");

    loop {
        let prompt = {
            let sess = session.lock().await;
            if let Some(username) = &sess.username {
                format!("{username}> ")
            } else {
                "conclave> ".to_string()
            }
        };

        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                rl.add_history_entry(line)
                    .map_err(|e| Error::Other(e.to_string()))?;

                if let Err(e) =
                    handle_command(line, &api, &session, &config_data_dir, &group_mapping).await
                {
                    eprintln!("Error: {e}");
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                println!("Goodbye.");
                break;
            }
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_command(
    input: &str,
    api: &Arc<Mutex<ApiClient>>,
    session: &Arc<Mutex<SessionState>>,
    data_dir: &std::path::Path,
    group_mapping: &Arc<Mutex<GroupMapping>>,
) -> Result<()> {
    let parts: Vec<&str> = input.splitn(3, ' ').collect();
    let cmd = parts[0];

    match cmd {
        "/help" => {
            println!("Commands:");
            println!("  /register <username> <password>  - Register a new account");
            println!("  /login <username> <password>     - Login to the server");
            println!("  /me                              - Show current user info");
            println!("  /keygen                          - Generate and upload a key package");
            println!(
                "  /create <name> <members...>      - Create a group (comma-separated members)"
            );
            println!("  /invite <group_id> <members...>  - Invite members to an existing group");
            println!("  /groups                          - List your groups");
            println!("  /join                             - Accept pending group invitations");
            println!("  /send <group_id> <message>       - Send a message to a group");
            println!("  /messages <group_id>             - Fetch messages from a group");
            println!("  /quit                            - Exit");
        }

        "/register" => {
            if parts.len() < 3 {
                println!("Usage: /register <username> <password>");
                return Ok(());
            }
            let username = parts[1];
            let password = parts[2];
            let resp = api.lock().await.register(username, password).await?;
            println!("Registered as user ID {}", resp.user_id);
        }

        "/login" => {
            if parts.len() < 3 {
                println!("Usage: /login <username> <password>");
                return Ok(());
            }
            let username = parts[1];
            let password = parts[2];
            let resp = api.lock().await.login(username, password).await?;

            {
                let mut api_guard = api.lock().await;
                api_guard.set_token(resp.token.clone());
            }

            {
                let mut sess = session.lock().await;
                sess.token = Some(resp.token);
                sess.user_id = Some(resp.user_id);
                sess.username = Some(username.to_string());
                sess.save(&data_dir.to_path_buf())?;
            }

            // Initialize MLS identity.
            std::fs::create_dir_all(data_dir)?;
            let _mls = MlsManager::new(data_dir, username)?;

            // Reload group mapping for this user.
            {
                let mut mapping = group_mapping.lock().await;
                *mapping = load_group_mapping(&data_dir.join("users").join(username));
            }

            println!("Logged in as {username} (user ID {})", resp.user_id);
        }

        "/me" => {
            let resp = api.lock().await.me().await?;
            println!("User: {} (ID: {})", resp.username, resp.user_id);
        }

        "/keygen" => {
            let sess = session.lock().await;
            let username = sess
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(data_dir, username)?;
            let kp = mls.generate_key_package()?;
            drop(sess);

            api.lock().await.upload_key_package(kp).await?;
            println!("Key package generated and uploaded.");
        }

        "/create" => {
            if parts.len() < 3 {
                println!("Usage: /create <group_name> <member1,member2,...>");
                return Ok(());
            }
            let group_name = parts[1];
            let member_names: Vec<String> =
                parts[2].split(',').map(|s| s.trim().to_string()).collect();

            let resp = api
                .lock()
                .await
                .create_group(group_name, member_names)
                .await?;

            let server_group_id = resp.group_id.clone();

            // Create MLS group locally.
            let sess = session.lock().await;
            let username = sess
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(data_dir, username)?;
            drop(sess);

            let (mls_group_id, commit_bytes, welcome_map, group_info_bytes) =
                mls.create_group(&resp.member_key_packages)?;

            // Upload commit + welcomes to server.
            api.lock()
                .await
                .upload_commit(
                    &server_group_id,
                    commit_bytes,
                    welcome_map,
                    group_info_bytes,
                )
                .await?;

            // Save the group mapping.
            {
                let mut mapping = group_mapping.lock().await;
                mapping.insert(server_group_id.clone(), mls_group_id);
                save_group_mapping(mls.user_data_dir(), &mapping);
            }

            println!("Group '{group_name}' created (ID: {server_group_id})");
        }

        "/invite" => {
            if parts.len() < 3 {
                println!("Usage: /invite <group_id> <member1,member2,...>");
                return Ok(());
            }
            let server_group_id = parts[1];
            let member_names: Vec<String> =
                parts[2].split(',').map(|s| s.trim().to_string()).collect();

            let resp = api
                .lock()
                .await
                .invite_to_group(server_group_id, member_names)
                .await?;

            if resp.member_key_packages.is_empty() {
                println!("No new members to invite.");
                return Ok(());
            }

            let mls_group_id = {
                let mapping = group_mapping.lock().await;
                mapping.get(server_group_id).cloned().ok_or_else(|| {
                    Error::Other(format!(
                        "unknown group '{server_group_id}' — try /join first"
                    ))
                })?
            };

            let sess = session.lock().await;
            let username = sess
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(data_dir, username)?;
            drop(sess);

            let (commit_bytes, welcome_map, group_info_bytes) =
                mls.invite_to_group(&mls_group_id, &resp.member_key_packages)?;

            api.lock()
                .await
                .upload_commit(server_group_id, commit_bytes, welcome_map, group_info_bytes)
                .await?;

            let invited: Vec<String> = resp.member_key_packages.keys().cloned().collect();
            println!("Invited {} to group {server_group_id}", invited.join(", "));
        }

        "/groups" => {
            let resp = api.lock().await.list_groups().await?;
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

        "/join" => {
            let resp = api.lock().await.list_pending_welcomes().await?;
            if resp.welcomes.is_empty() {
                println!("No pending invitations.");
                return Ok(());
            }

            let sess = session.lock().await;
            let username = sess
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(data_dir, username)?;
            drop(sess);

            for welcome in &resp.welcomes {
                let mls_group_id = mls.join_group(&welcome.welcome_message)?;

                {
                    let mut mapping = group_mapping.lock().await;
                    mapping.insert(welcome.group_id.clone(), mls_group_id);
                    save_group_mapping(mls.user_data_dir(), &mapping);
                }

                println!(
                    "Joined group '{}' (ID: {})",
                    welcome.group_name, welcome.group_id
                );
            }
        }

        "/send" => {
            if parts.len() < 3 {
                println!("Usage: /send <group_id> <message>");
                return Ok(());
            }
            let server_group_id = parts[1];
            let message_text = parts[2];

            let mls_group_id = {
                let mapping = group_mapping.lock().await;
                mapping.get(server_group_id).cloned().ok_or_else(|| {
                    Error::Other(format!(
                        "unknown group '{server_group_id}' — try /join first"
                    ))
                })?
            };

            let sess = session.lock().await;
            let username = sess
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(data_dir, username)?;
            drop(sess);

            let encrypted = mls.encrypt_message(&mls_group_id, message_text.as_bytes())?;

            let resp = api
                .lock()
                .await
                .send_message(server_group_id, encrypted)
                .await?;

            println!("Message sent (seq: {})", resp.sequence_num);
        }

        "/messages" => {
            if parts.len() < 2 {
                println!("Usage: /messages <group_id>");
                return Ok(());
            }
            let server_group_id = parts[1];

            let mls_group_id = {
                let mapping = group_mapping.lock().await;
                mapping.get(server_group_id).cloned().ok_or_else(|| {
                    Error::Other(format!(
                        "unknown group '{server_group_id}' — try /join first"
                    ))
                })?
            };

            let sess = session.lock().await;
            let username = sess
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let mls = MlsManager::new(data_dir, username)?;
            drop(sess);

            let resp = api.lock().await.get_messages(server_group_id, 0).await?;

            if resp.messages.is_empty() {
                println!("No messages.");
            } else {
                for msg in &resp.messages {
                    match mls.decrypt_message(&mls_group_id, &msg.mls_message) {
                        Ok(Some(plaintext)) => {
                            let text = String::from_utf8_lossy(&plaintext);
                            println!("  [{}] {}: {}", msg.sequence_num, msg.sender_username, text);
                        }
                        Ok(None) => {
                            // Commit or proposal message, skip display.
                        }
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

        "/quit" | "/exit" => {
            println!("Goodbye.");
            std::process::exit(0);
        }

        _ => {
            println!("Unknown command. Type /help for available commands.");
        }
    }

    Ok(())
}

fn load_group_mapping(data_dir: &std::path::Path) -> GroupMapping {
    let path = data_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

fn save_group_mapping(data_dir: &std::path::Path, mapping: &GroupMapping) {
    let path = data_dir.join("group_mapping.toml");
    if let Ok(contents) = toml::to_string_pretty(mapping) {
        let _ = std::fs::write(path, contents);
    }
}
