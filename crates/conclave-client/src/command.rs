use uuid::Uuid;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Account,
    Rooms,
    Members,
    Invites,
    Security,
    General,
}

impl CommandCategory {
    pub fn label(self) -> &'static str {
        match self {
            CommandCategory::Account => "ACCOUNT",
            CommandCategory::Rooms => "ROOMS",
            CommandCategory::Members => "MEMBERS",
            CommandCategory::Invites => "INVITES",
            CommandCategory::Security => "SECURITY",
            CommandCategory::General => "GENERAL",
        }
    }

    const ALL: &[CommandCategory] = &[
        CommandCategory::Account,
        CommandCategory::Rooms,
        CommandCategory::Members,
        CommandCategory::Invites,
        CommandCategory::Security,
        CommandCategory::General,
    ];
}

pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub category: CommandCategory,
    pub usage: &'static str,
    pub description: &'static str,
}

pub struct HelpCategory {
    pub label: &'static str,
    pub commands: Vec<&'static CommandSpec>,
}

pub static COMMANDS: &[CommandSpec] = &[
    // Account
    CommandSpec {
        name: "register",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/register <server> <username> [token]",
        description: "Register a new account and login",
    },
    CommandSpec {
        name: "login",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/login <server> <username>",
        description: "Login to the server",
    },
    CommandSpec {
        name: "logout",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/logout",
        description: "Logout and revoke session",
    },
    CommandSpec {
        name: "reset",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/reset",
        description: "Reset account and rejoin groups",
    },
    CommandSpec {
        name: "alias",
        aliases: &["nick"],
        category: CommandCategory::Account,
        usage: "/alias <name>",
        description: "Set your display name",
    },
    CommandSpec {
        name: "passwd",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/passwd",
        description: "Change your password",
    },
    CommandSpec {
        name: "expunge",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/expunge",
        description: "Permanently delete your account and all data",
    },
    // Rooms
    CommandSpec {
        name: "create",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/create <name>",
        description: "Create a new room",
    },
    CommandSpec {
        name: "rooms",
        aliases: &["list"],
        category: CommandCategory::Rooms,
        usage: "/rooms",
        description: "List your rooms",
    },
    CommandSpec {
        name: "join",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/join [room]",
        description: "Accept invitations, switch to a room, or join a public room",
    },
    CommandSpec {
        name: "close",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/close",
        description: "Switch away without leaving",
    },
    CommandSpec {
        name: "part",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/part",
        description: "Leave the room (MLS removal)",
    },
    CommandSpec {
        name: "info",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/info",
        description: "Show MLS group details",
    },
    CommandSpec {
        name: "topic",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/topic <text>",
        description: "Set active room's display name",
    },
    CommandSpec {
        name: "unread",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/unread",
        description: "Check rooms for new messages",
    },
    CommandSpec {
        name: "expire",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/expire [duration]",
        description: "Set or view message expiration policy",
    },
    CommandSpec {
        name: "delete",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/delete",
        description: "Delete the active room (admin only)",
    },
    CommandSpec {
        name: "visibility",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/visibility [public|private]",
        description: "Show or set room visibility (admin only to set)",
    },
    CommandSpec {
        name: "discover",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/discover [pattern]",
        description: "List public rooms",
    },
    // Members
    CommandSpec {
        name: "members",
        aliases: &["who"],
        category: CommandCategory::Members,
        usage: "/members",
        description: "List members of active room",
    },
    CommandSpec {
        name: "whois",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/whois [username]",
        description: "Show user info and fingerprint",
    },
    CommandSpec {
        name: "invite",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/invite <user1,user2>",
        description: "Invite to the active room",
    },
    CommandSpec {
        name: "kick",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/kick <username>",
        description: "Remove a member from the room",
    },
    CommandSpec {
        name: "promote",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/promote <username>",
        description: "Promote a member to admin",
    },
    CommandSpec {
        name: "demote",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/demote <username>",
        description: "Demote an admin to member",
    },
    CommandSpec {
        name: "admins",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/admins",
        description: "List admins of active room",
    },
    CommandSpec {
        name: "invited",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/invited",
        description: "List pending invites for active room",
    },
    CommandSpec {
        name: "uninvite",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/uninvite <username>",
        description: "Cancel a pending invite",
    },
    CommandSpec {
        name: "ban",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/ban <username>",
        description: "Ban a member from the room",
    },
    CommandSpec {
        name: "unban",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/unban <username>",
        description: "Unban a user from the room",
    },
    CommandSpec {
        name: "banned",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/banned",
        description: "List banned users in the room",
    },
    // Invites
    CommandSpec {
        name: "invites",
        aliases: &[],
        category: CommandCategory::Invites,
        usage: "/invites",
        description: "List pending invitations",
    },
    CommandSpec {
        name: "accept",
        aliases: &[],
        category: CommandCategory::Invites,
        usage: "/accept [id]",
        description: "Accept a pending invite (or all)",
    },
    CommandSpec {
        name: "decline",
        aliases: &[],
        category: CommandCategory::Invites,
        usage: "/decline <id>",
        description: "Decline a pending invite",
    },
    // Security
    CommandSpec {
        name: "rotate",
        aliases: &[],
        category: CommandCategory::Security,
        usage: "/rotate",
        description: "Rotate keys (forward secrecy)",
    },
    CommandSpec {
        name: "trusted",
        aliases: &[],
        category: CommandCategory::Security,
        usage: "/trusted",
        description: "List all known user fingerprints",
    },
    CommandSpec {
        name: "unverify",
        aliases: &[],
        category: CommandCategory::Security,
        usage: "/unverify <username>",
        description: "Remove verification for a user's signing key",
    },
    CommandSpec {
        name: "verify",
        aliases: &[],
        category: CommandCategory::Security,
        usage: "/verify <user> <fingerprint>",
        description: "Verify a user's signing key fingerprint",
    },
    // General
    CommandSpec {
        name: "help",
        aliases: &["h"],
        category: CommandCategory::General,
        usage: "/help",
        description: "Show this help",
    },
    CommandSpec {
        name: "quit",
        aliases: &["exit", "q"],
        category: CommandCategory::General,
        usage: "/quit",
        description: "Exit",
    },
];

pub fn help_categories() -> Vec<HelpCategory> {
    CommandCategory::ALL
        .iter()
        .map(|category| {
            let mut commands: Vec<&CommandSpec> = COMMANDS
                .iter()
                .filter(|spec| spec.category == *category)
                .collect();
            commands.sort_by_key(|spec| spec.name);
            HelpCategory {
                label: category.label(),
                commands,
            }
        })
        .filter(|help_category| !help_category.commands.is_empty())
        .collect()
}

pub fn format_help_lines() -> Vec<String> {
    let categories = help_categories();

    let max_usage_width = COMMANDS
        .iter()
        .map(|spec| spec.usage.len())
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    for (index, category) in categories.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.push(format!("{}:", category.label));
        for spec in &category.commands {
            lines.push(format!(
                "  {:<width$}  {}",
                spec.usage,
                spec.description,
                width = max_usage_width,
            ));
        }
    }

    lines.push(String::new());
    lines.push("Type text without / to send a message to the active room.".to_string());
    lines
}

fn lookup_command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|spec| spec.name == name || spec.aliases.contains(&name))
}

/// Parsed command from user input.
pub enum Command {
    // Account
    Register {
        server: String,
        username: String,
        token: Option<String>,
    },
    Login {
        server: String,
        username: String,
    },
    Logout,
    Reset,
    Alias {
        alias: String,
    },
    Whois {
        username: Option<String>,
    },
    Verify {
        username: String,
        fingerprint: String,
    },
    Unverify {
        username: String,
    },
    Trusted,
    Passwd,
    Expunge {
        password: Option<String>,
    },

    // Rooms
    Create {
        name: String,
    },
    /// No args: accept pending welcomes. With arg: switch to room.
    Join {
        target: Option<String>,
    },
    Rooms,
    Close,
    Part,
    Info,
    Topic {
        topic: String,
    },
    Unread,
    Expire {
        duration: Option<String>,
    },
    Delete,
    Visibility {
        visibility: Option<String>,
    },
    Discover {
        pattern: Option<String>,
    },

    // Members
    Members,
    Invite {
        members: Vec<String>,
    },
    Kick {
        username: String,
    },
    Promote {
        username: String,
    },
    Demote {
        username: String,
    },
    Admins,
    Invited,
    Uninvite {
        username: String,
    },
    Ban {
        username: String,
    },
    Unban {
        username: String,
    },
    Banned,

    // Invites
    Invites,
    Accept {
        invite_id: Option<Uuid>,
    },
    Decline {
        invite_id: Uuid,
    },

    // Security
    Rotate,

    // Plain text (not a slash command)
    Message {
        text: String,
    },

    // General
    Help,
    Quit,
}

/// Parse user input into a Command.
pub fn parse(input: &str) -> Result<Command> {
    if !input.starts_with('/') {
        return Ok(Command::Message {
            text: input.to_string(),
        });
    }

    let parts: Vec<&str> = input.splitn(3, ' ').collect();
    let cmd_name = parts[0].strip_prefix('/').unwrap_or(parts[0]);

    let Some(spec) = lookup_command(cmd_name) else {
        return Err(Error::Other(format!(
            "Unknown command: /{cmd_name}. Type /help for available commands."
        )));
    };

    parse_command_args(spec.name, &parts, input)
}

fn parse_command_args(name: &str, parts: &[&str], full_input: &str) -> Result<Command> {
    let cmd_word = parts[0];
    match name {
        // Account
        "register" => {
            let parts: Vec<&str> = full_input.splitn(4, ' ').collect();
            match parts.len() {
                3 => Ok(Command::Register {
                    server: parts[1].to_string(),
                    username: parts[2].to_string(),
                    token: None,
                }),
                4 => Ok(Command::Register {
                    server: parts[1].to_string(),
                    username: parts[2].to_string(),
                    token: Some(parts[3].to_string()),
                }),
                _ => Err(Error::Other(
                    "Usage: /register <server> <username> [token]".into(),
                )),
            }
        }
        "login" => {
            if parts.len() < 3 {
                return Err(Error::Other("Usage: /login <server> <username>".into()));
            }
            Ok(Command::Login {
                server: parts[1].to_string(),
                username: parts[2].to_string(),
            })
        }
        "logout" => Ok(Command::Logout),
        "reset" => Ok(Command::Reset),
        "alias" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /alias <name>".into()));
            }
            let prefix = format!("{cmd_word} ");
            let alias = full_input[prefix.len()..].to_string();
            Ok(Command::Alias { alias })
        }
        "whois" => {
            let username = parts.get(1).map(|s| s.to_string());
            Ok(Command::Whois { username })
        }
        "verify" => {
            if parts.len() < 2 {
                return Err(Error::Other(
                    "Usage: /verify <username> <fingerprint>".into(),
                ));
            }
            let username = parts[1].to_string();
            let prefix = format!("{cmd_word} {username} ");
            if full_input.len() <= prefix.len() {
                return Err(Error::Other(
                    "Usage: /verify <username> <fingerprint>".into(),
                ));
            }
            let fingerprint = full_input[prefix.len()..].to_string();
            Ok(Command::Verify {
                username,
                fingerprint,
            })
        }
        "unverify" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /unverify <username>".into()));
            }
            Ok(Command::Unverify {
                username: parts[1].to_string(),
            })
        }
        "trusted" => Ok(Command::Trusted),
        "passwd" => Ok(Command::Passwd),
        "expunge" => Ok(Command::Expunge { password: None }),

        // Rooms
        "create" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /create <name>".into()));
            }
            Ok(Command::Create {
                name: parts[1].to_string(),
            })
        }
        "join" => {
            let target = parts.get(1).map(|s| s.to_string());
            Ok(Command::Join { target })
        }
        "rooms" => Ok(Command::Rooms),
        "close" => Ok(Command::Close),
        "part" => Ok(Command::Part),
        "info" => Ok(Command::Info),
        "topic" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /topic <text>".into()));
            }
            let prefix = format!("{cmd_word} ");
            let topic = full_input[prefix.len()..].to_string();
            Ok(Command::Topic { topic })
        }
        "unread" => Ok(Command::Unread),
        "delete" => Ok(Command::Delete),
        "expire" => {
            let duration = if parts.len() >= 2 {
                let prefix = format!("{cmd_word} ");
                Some(full_input[prefix.len()..].to_string())
            } else {
                None
            };
            Ok(Command::Expire { duration })
        }

        "visibility" => {
            if parts.len() < 2 {
                return Ok(Command::Visibility { visibility: None });
            }
            let value = parts[1].to_lowercase();
            if value != "public" && value != "private" {
                return Err(Error::Other("Usage: /visibility [public|private]".into()));
            }
            Ok(Command::Visibility {
                visibility: Some(value),
            })
        }
        "discover" => {
            let pattern = parts.get(1).map(|s| s.to_string());
            Ok(Command::Discover { pattern })
        }

        // Members
        "members" => Ok(Command::Members),
        "invite" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /invite <member1,member2,...>".into()));
            }
            let members = parts[1].split(',').map(|s| s.trim().to_string()).collect();
            Ok(Command::Invite { members })
        }
        "kick" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /kick <username>".into()));
            }
            Ok(Command::Kick {
                username: parts[1].to_string(),
            })
        }
        "promote" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /promote <username>".into()));
            }
            Ok(Command::Promote {
                username: parts[1].to_string(),
            })
        }
        "demote" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /demote <username>".into()));
            }
            Ok(Command::Demote {
                username: parts[1].to_string(),
            })
        }
        "admins" => Ok(Command::Admins),
        "invited" => Ok(Command::Invited),
        "uninvite" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /uninvite <username>".into()));
            }
            Ok(Command::Uninvite {
                username: parts[1].to_string(),
            })
        }
        "ban" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /ban <username>".into()));
            }
            Ok(Command::Ban {
                username: parts[1].to_string(),
            })
        }
        "unban" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /unban <username>".into()));
            }
            Ok(Command::Unban {
                username: parts[1].to_string(),
            })
        }
        "banned" => Ok(Command::Banned),

        // Invites
        "invites" => Ok(Command::Invites),
        "accept" => {
            let invite_id = parts.get(1).map(|s| {
                Uuid::parse_str(s).map_err(|_| Error::Other("Usage: /accept [invite_id]".into()))
            });
            match invite_id {
                Some(Ok(id)) => Ok(Command::Accept {
                    invite_id: Some(id),
                }),
                Some(Err(e)) => Err(e),
                None => Ok(Command::Accept { invite_id: None }),
            }
        }
        "decline" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /decline <invite_id>".into()));
            }
            let invite_id = Uuid::parse_str(parts[1])
                .map_err(|_| Error::Other("Usage: /decline <invite_id>".into()))?;
            Ok(Command::Decline { invite_id })
        }

        // Security
        "rotate" => Ok(Command::Rotate),

        // General
        "help" => Ok(Command::Help),
        "quit" => Ok(Command::Quit),
        _ => Err(Error::Other(format!(
            "Unknown command: /{name}. Type /help for available commands."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Account

    #[test]
    fn test_parse_register() {
        let cmd = parse("/register example.com alice").unwrap();
        let Command::Register {
            server,
            username,
            token,
        } = cmd
        else {
            panic!("expected Register variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "alice");
        assert_eq!(token, None);
    }

    #[test]
    fn test_parse_register_with_token() {
        let cmd = parse("/register example.com alice mytoken").unwrap();
        let Command::Register {
            server,
            username,
            token,
        } = cmd
        else {
            panic!("expected Register variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "alice");
        assert_eq!(token, Some("mytoken".to_string()));
    }

    #[test]
    fn test_parse_register_missing_args() {
        assert!(parse("/register example.com").is_err());
        assert!(parse("/register").is_err());
    }

    #[test]
    fn test_parse_login() {
        let cmd = parse("/login example.com alice").unwrap();
        let Command::Login { server, username } = cmd else {
            panic!("expected Login variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_login_missing_args() {
        assert!(parse("/login example.com").is_err());
        assert!(parse("/login").is_err());
    }

    #[test]
    fn test_parse_passwd() {
        let cmd = parse("/passwd").unwrap();
        assert!(matches!(cmd, Command::Passwd));
    }

    #[test]
    fn test_parse_passwd_ignores_extra_args() {
        let cmd = parse("/passwd extra args").unwrap();
        assert!(matches!(cmd, Command::Passwd));
    }

    #[test]
    fn test_parse_expunge() {
        let cmd = parse("/expunge").unwrap();
        assert!(matches!(cmd, Command::Expunge { password: None }));
    }

    // Rooms

    #[test]
    fn test_parse_create() {
        let cmd = parse("/create room").unwrap();
        let Command::Create { name } = cmd else {
            panic!("expected Create variant");
        };
        assert_eq!(name, "room");
    }

    #[test]
    fn test_parse_create_missing_args() {
        assert!(parse("/create").is_err());
    }

    #[test]
    fn test_parse_join_no_arg() {
        let cmd = parse("/join").unwrap();
        let Command::Join { target } = cmd else {
            panic!("expected Join variant");
        };
        assert!(target.is_none());
    }

    #[test]
    fn test_parse_join_with_target() {
        let cmd = parse("/join myroom").unwrap();
        let Command::Join { target } = cmd else {
            panic!("expected Join variant");
        };
        assert_eq!(target, Some("myroom".to_string()));
    }

    // Members

    #[test]
    fn test_parse_invite() {
        let cmd = parse("/invite alice,bob").unwrap();
        let Command::Invite { members } = cmd else {
            panic!("expected Invite variant");
        };
        assert_eq!(members, vec!["alice", "bob"]);
    }

    #[test]
    fn test_parse_invite_missing_args() {
        assert!(parse("/invite").is_err());
    }

    #[test]
    fn test_parse_kick() {
        let cmd = parse("/kick alice").unwrap();
        let Command::Kick { username } = cmd else {
            panic!("expected Kick variant");
        };
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_kick_missing_args() {
        assert!(parse("/kick").is_err());
    }

    #[test]
    fn test_parse_part() {
        let cmd = parse("/part").unwrap();
        assert!(matches!(cmd, Command::Part));
    }

    #[test]
    fn test_parse_close() {
        let cmd = parse("/close").unwrap();
        assert!(matches!(cmd, Command::Close));
    }

    #[test]
    fn test_parse_rotate() {
        let cmd = parse("/rotate").unwrap();
        assert!(matches!(cmd, Command::Rotate));
    }

    #[test]
    fn test_parse_reset() {
        let cmd = parse("/reset").unwrap();
        assert!(matches!(cmd, Command::Reset));
    }

    #[test]
    fn test_parse_info() {
        let cmd = parse("/info").unwrap();
        assert!(matches!(cmd, Command::Info));
    }

    #[test]
    fn test_parse_rooms() {
        let cmd = parse("/rooms").unwrap();
        assert!(matches!(cmd, Command::Rooms));
    }

    #[test]
    fn test_parse_rooms_alias() {
        let cmd = parse("/list").unwrap();
        assert!(matches!(cmd, Command::Rooms));
    }

    #[test]
    fn test_parse_members() {
        let cmd = parse("/members").unwrap();
        assert!(matches!(cmd, Command::Members));
    }

    #[test]
    fn test_parse_members_alias() {
        let cmd = parse("/who").unwrap();
        assert!(matches!(cmd, Command::Members));
    }

    // Plain text

    #[test]
    fn test_parse_plain_message() {
        let cmd = parse("hello").unwrap();
        let Command::Message { text } = cmd else {
            panic!("expected Message variant");
        };
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_parse_unread() {
        let cmd = parse("/unread").unwrap();
        assert!(matches!(cmd, Command::Unread));
    }

    #[test]
    fn test_parse_logout() {
        let cmd = parse("/logout").unwrap();
        assert!(matches!(cmd, Command::Logout));
    }

    #[test]
    fn test_parse_whois_no_arg() {
        let cmd = parse("/whois").unwrap();
        let Command::Whois { username } = cmd else {
            panic!("expected Whois variant");
        };
        assert!(username.is_none());
    }

    #[test]
    fn test_parse_whois_with_username() {
        let cmd = parse("/whois alice").unwrap();
        let Command::Whois { username } = cmd else {
            panic!("expected Whois variant");
        };
        assert_eq!(username, Some("alice".to_string()));
    }

    #[test]
    fn test_parse_verify() {
        let cmd = parse("/verify alice a1b2c3d4 e5f6a7b8").unwrap();
        let Command::Verify {
            username,
            fingerprint,
        } = cmd
        else {
            panic!("expected Verify variant");
        };
        assert_eq!(username, "alice");
        assert_eq!(fingerprint, "a1b2c3d4 e5f6a7b8");
    }

    #[test]
    fn test_parse_verify_missing_args() {
        assert!(parse("/verify").is_err());
        assert!(parse("/verify alice").is_err());
    }

    #[test]
    fn test_parse_unverify() {
        let cmd = parse("/unverify alice").unwrap();
        let Command::Unverify { username } = cmd else {
            panic!("expected Unverify variant");
        };
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_unverify_missing_args() {
        assert!(parse("/unverify").is_err());
    }

    #[test]
    fn test_parse_trusted() {
        let cmd = parse("/trusted").unwrap();
        assert!(matches!(cmd, Command::Trusted));
    }

    // General

    #[test]
    fn test_parse_unknown_command() {
        assert!(parse("/xyz").is_err());
    }

    #[test]
    fn test_parse_help() {
        let cmd = parse("/help").unwrap();
        assert!(matches!(cmd, Command::Help));
    }

    #[test]
    fn test_parse_help_alias() {
        let cmd = parse("/h").unwrap();
        assert!(matches!(cmd, Command::Help));
    }

    #[test]
    fn test_parse_quit() {
        let cmd = parse("/quit").unwrap();
        assert!(matches!(cmd, Command::Quit));
    }

    #[test]
    fn test_parse_quit_aliases() {
        let cmd_exit = parse("/exit").unwrap();
        assert!(matches!(cmd_exit, Command::Quit));

        let cmd_q = parse("/q").unwrap();
        assert!(matches!(cmd_q, Command::Quit));
    }

    #[test]
    fn test_parse_alias() {
        let cmd = parse("/alias Alice Smith").unwrap();
        let Command::Alias { alias } = cmd else {
            panic!("expected Alias variant");
        };
        assert_eq!(alias, "Alice Smith");
    }

    #[test]
    fn test_parse_alias_alias() {
        let cmd = parse("/nick Alice Smith").unwrap();
        let Command::Alias { alias } = cmd else {
            panic!("expected Alias variant");
        };
        assert_eq!(alias, "Alice Smith");
    }

    #[test]
    fn test_parse_alias_missing_args() {
        assert!(parse("/alias").is_err());
    }

    #[test]
    fn test_parse_login_extra_args_in_username() {
        // Extra args after username are treated as part of the username
        // since /login only has 2 positional args.
        let cmd = parse("/login example.com alice extra").unwrap();
        let Command::Login { server, username } = cmd else {
            panic!("expected Login variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "alice extra");
    }

    // Rooms (continued)

    #[test]
    fn test_parse_topic() {
        let cmd = parse("/topic Dev Team Chat").unwrap();
        let Command::Topic { topic } = cmd else {
            panic!("expected Topic variant");
        };
        assert_eq!(topic, "Dev Team Chat");
    }

    #[test]
    fn test_parse_topic_missing_args() {
        assert!(parse("/topic").is_err());
    }

    // Members (continued)

    #[test]
    fn test_parse_admins() {
        let cmd = parse("/admins").unwrap();
        assert!(matches!(cmd, Command::Admins));
    }

    #[test]
    fn test_parse_promote() {
        let cmd = parse("/promote alice").unwrap();
        let Command::Promote { username } = cmd else {
            panic!("expected Promote variant");
        };
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_promote_missing_username() {
        assert!(parse("/promote").is_err());
    }

    #[test]
    fn test_parse_demote() {
        let cmd = parse("/demote bob").unwrap();
        let Command::Demote { username } = cmd else {
            panic!("expected Demote variant");
        };
        assert_eq!(username, "bob");
    }

    #[test]
    fn test_parse_demote_missing_username() {
        assert!(parse("/demote").is_err());
    }

    #[test]
    fn test_parse_invited() {
        let cmd = parse("/invited").unwrap();
        assert!(matches!(cmd, Command::Invited));
    }

    #[test]
    fn test_parse_uninvite() {
        let cmd = parse("/uninvite alice").unwrap();
        let Command::Uninvite { username } = cmd else {
            panic!("expected Uninvite variant");
        };
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_uninvite_missing_args() {
        assert!(parse("/uninvite").is_err());
    }

    // Invites

    #[test]
    fn test_parse_invites() {
        let cmd = parse("/invites").unwrap();
        assert!(matches!(cmd, Command::Invites));
    }

    #[test]
    fn test_parse_accept_no_arg() {
        let cmd = parse("/accept").unwrap();
        let Command::Accept { invite_id } = cmd else {
            panic!("expected Accept variant");
        };
        assert!(invite_id.is_none());
    }

    #[test]
    fn test_parse_accept_with_id() {
        let test_uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cmd = parse(&format!("/accept {test_uuid}")).unwrap();
        let Command::Accept { invite_id } = cmd else {
            panic!("expected Accept variant");
        };
        assert_eq!(invite_id, Some(test_uuid));
    }

    #[test]
    fn test_parse_accept_invalid_id() {
        assert!(parse("/accept not-a-uuid").is_err());
    }

    #[test]
    fn test_parse_decline() {
        let test_uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cmd = parse(&format!("/decline {test_uuid}")).unwrap();
        let Command::Decline { invite_id } = cmd else {
            panic!("expected Decline variant");
        };
        assert_eq!(invite_id, test_uuid);
    }

    #[test]
    fn test_parse_decline_missing_id() {
        assert!(parse("/decline").is_err());
    }

    #[test]
    fn test_parse_decline_invalid_id() {
        assert!(parse("/decline xyz").is_err());
    }

    // Registry validation

    #[test]
    fn test_registry_names_unique() {
        let mut seen = std::collections::HashSet::new();
        for spec in COMMANDS {
            assert!(
                seen.insert(spec.name),
                "duplicate command name: {}",
                spec.name
            );
            for alias in spec.aliases {
                assert!(seen.insert(alias), "duplicate alias: {alias}");
            }
        }
    }

    #[test]
    fn test_registry_commands_parseable() {
        for spec in COMMANDS {
            let input = format!("/{}", spec.name);
            if let Err(ref error) = parse(&input) {
                let message = error.to_string();
                assert!(
                    !message.contains("Unknown command"),
                    "command /{} not recognized by parser",
                    spec.name,
                );
            }
        }
    }

    #[test]
    fn test_registry_aliases_parseable() {
        for spec in COMMANDS {
            for alias in spec.aliases {
                let input = format!("/{alias}");
                if let Err(ref error) = parse(&input) {
                    let message = error.to_string();
                    assert!(
                        !message.contains("Unknown command"),
                        "alias /{alias} for /{} not recognized by parser",
                        spec.name,
                    );
                }
            }
        }
    }

    #[test]
    fn test_help_categories_non_empty() {
        let categories = help_categories();
        assert!(!categories.is_empty());
        for category in &categories {
            assert!(
                !category.commands.is_empty(),
                "empty category: {}",
                category.label,
            );
        }
    }

    #[test]
    fn test_parse_visibility_public() {
        let cmd = parse("/visibility public").unwrap();
        let Command::Visibility { visibility } = cmd else {
            panic!("expected Visibility variant");
        };
        assert_eq!(visibility, Some("public".to_string()));
    }

    #[test]
    fn test_parse_visibility_private() {
        let cmd = parse("/visibility private").unwrap();
        let Command::Visibility { visibility } = cmd else {
            panic!("expected Visibility variant");
        };
        assert_eq!(visibility, Some("private".to_string()));
    }

    #[test]
    fn test_parse_visibility_invalid() {
        assert!(parse("/visibility open").is_err());
    }

    #[test]
    fn test_parse_visibility_no_arg() {
        let cmd = parse("/visibility").unwrap();
        let Command::Visibility { visibility } = cmd else {
            panic!("expected Visibility variant");
        };
        assert!(visibility.is_none());
    }

    #[test]
    fn test_parse_discover_no_pattern() {
        let cmd = parse("/discover").unwrap();
        let Command::Discover { pattern } = cmd else {
            panic!("expected Discover variant");
        };
        assert!(pattern.is_none());
    }

    #[test]
    fn test_parse_discover_with_pattern() {
        let cmd = parse("/discover dev").unwrap();
        let Command::Discover { pattern } = cmd else {
            panic!("expected Discover variant");
        };
        assert_eq!(pattern, Some("dev".to_string()));
    }

    #[test]
    fn test_parse_ban() {
        let cmd = parse("/ban alice").unwrap();
        let Command::Ban { username } = cmd else {
            panic!("expected Ban variant");
        };
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_ban_missing_arg() {
        assert!(parse("/ban").is_err());
    }

    #[test]
    fn test_parse_unban() {
        let cmd = parse("/unban alice").unwrap();
        let Command::Unban { username } = cmd else {
            panic!("expected Unban variant");
        };
        assert_eq!(username, "alice");
    }

    #[test]
    fn test_parse_unban_missing_arg() {
        assert!(parse("/unban").is_err());
    }

    #[test]
    fn test_parse_banned() {
        let cmd = parse("/banned").unwrap();
        assert!(matches!(cmd, Command::Banned));
    }

    #[test]
    fn test_format_help_contains_all_commands() {
        let lines = format_help_lines();
        let text = lines.join("\n");
        for spec in COMMANDS {
            assert!(
                text.contains(spec.usage),
                "help output missing usage for /{}",
                spec.name,
            );
        }
    }
}
