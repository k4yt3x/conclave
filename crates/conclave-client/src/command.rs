use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Account,
    Rooms,
    Members,
    Invites,
    Messaging,
    General,
}

impl CommandCategory {
    pub fn label(self) -> &'static str {
        match self {
            CommandCategory::Account => "ACCOUNT",
            CommandCategory::Rooms => "ROOMS",
            CommandCategory::Members => "MEMBERS",
            CommandCategory::Invites => "INVITES",
            CommandCategory::Messaging => "MESSAGING",
            CommandCategory::General => "GENERAL",
        }
    }

    const ALL: &[CommandCategory] = &[
        CommandCategory::Account,
        CommandCategory::Rooms,
        CommandCategory::Members,
        CommandCategory::Invites,
        CommandCategory::Messaging,
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
        usage: "/register <server> <user> <pass>",
        description: "Register a new account and login",
    },
    CommandSpec {
        name: "login",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/login <server> <user> <pass>",
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
        name: "nick",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/nick <alias>",
        description: "Set your display name",
    },
    CommandSpec {
        name: "whois",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/whois",
        description: "Show current user info",
    },
    CommandSpec {
        name: "passwd",
        aliases: &[],
        category: CommandCategory::Account,
        usage: "/passwd <new_password>",
        description: "Change your password",
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
        name: "list",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/list",
        description: "List your rooms",
    },
    CommandSpec {
        name: "join",
        aliases: &[],
        category: CommandCategory::Rooms,
        usage: "/join [room]",
        description: "Accept pending invitations or switch to a room",
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
    // Members
    CommandSpec {
        name: "who",
        aliases: &[],
        category: CommandCategory::Members,
        usage: "/who",
        description: "List members of active room",
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
    // Messaging
    CommandSpec {
        name: "msg",
        aliases: &[],
        category: CommandCategory::Messaging,
        usage: "/msg <room> <text>",
        description: "Send to a room without switching",
    },
    CommandSpec {
        name: "rotate",
        aliases: &[],
        category: CommandCategory::Messaging,
        usage: "/rotate",
        description: "Rotate keys (forward secrecy)",
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
        password: String,
    },
    Login {
        server: String,
        username: String,
        password: String,
    },
    Logout,
    Reset,
    Nick {
        alias: String,
    },
    Whois,
    Passwd {
        new_password: String,
    },

    // Rooms
    Create {
        name: String,
    },
    /// No args: accept pending welcomes. With arg: switch to room.
    Join {
        target: Option<String>,
    },
    List,
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

    // Members
    Who,
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

    // Invites
    Invites,
    Accept {
        invite_id: Option<i64>,
    },
    Decline {
        invite_id: i64,
    },

    // Messaging
    Msg {
        room: String,
        text: String,
    },
    Rotate,
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
    match name {
        // Account
        "register" => {
            let parts: Vec<&str> = full_input.splitn(4, ' ').collect();
            if parts.len() < 4 {
                return Err(Error::Other(
                    "Usage: /register <server> <username> <password>".into(),
                ));
            }
            Ok(Command::Register {
                server: parts[1].to_string(),
                username: parts[2].to_string(),
                password: parts[3].to_string(),
            })
        }
        "login" => {
            let parts: Vec<&str> = full_input.splitn(4, ' ').collect();
            if parts.len() < 4 {
                return Err(Error::Other(
                    "Usage: /login <server> <username> <password>".into(),
                ));
            }
            Ok(Command::Login {
                server: parts[1].to_string(),
                username: parts[2].to_string(),
                password: parts[3].to_string(),
            })
        }
        "logout" => Ok(Command::Logout),
        "reset" => Ok(Command::Reset),
        "nick" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /nick <alias>".into()));
            }
            let prefix = format!("/{name} ");
            let alias = full_input[prefix.len()..].to_string();
            Ok(Command::Nick { alias })
        }
        "whois" => Ok(Command::Whois),
        "passwd" => {
            let parts: Vec<&str> = full_input.splitn(2, ' ').collect();
            if parts.len() < 2 || parts[1].is_empty() {
                return Err(Error::Other("Usage: /passwd <new_password>".into()));
            }
            Ok(Command::Passwd {
                new_password: parts[1].to_string(),
            })
        }

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
        "list" => Ok(Command::List),
        "close" => Ok(Command::Close),
        "part" => Ok(Command::Part),
        "info" => Ok(Command::Info),
        "topic" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /topic <text>".into()));
            }
            let prefix = format!("/{name} ");
            let topic = full_input[prefix.len()..].to_string();
            Ok(Command::Topic { topic })
        }
        "unread" => Ok(Command::Unread),
        "expire" => {
            let duration = if parts.len() >= 2 {
                let prefix = format!("/{name} ");
                Some(full_input[prefix.len()..].to_string())
            } else {
                None
            };
            Ok(Command::Expire { duration })
        }

        // Members
        "who" => Ok(Command::Who),
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

        // Invites
        "invites" => Ok(Command::Invites),
        "accept" => {
            let invite_id = parts.get(1).map(|s| {
                s.parse::<i64>()
                    .map_err(|_| Error::Other("Usage: /accept [invite_id]".into()))
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
            let invite_id = parts[1]
                .parse::<i64>()
                .map_err(|_| Error::Other("Usage: /decline <invite_id>".into()))?;
            Ok(Command::Decline { invite_id })
        }

        // Messaging
        "msg" => {
            if parts.len() < 3 {
                return Err(Error::Other("Usage: /msg <room> <message>".into()));
            }
            Ok(Command::Msg {
                room: parts[1].to_string(),
                text: parts[2].to_string(),
            })
        }
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
        let cmd = parse("/register example.com alice pass1234").unwrap();
        let Command::Register {
            server,
            username,
            password,
        } = cmd
        else {
            panic!("expected Register variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "alice");
        assert_eq!(password, "pass1234");
    }

    #[test]
    fn test_parse_register_missing_args() {
        assert!(parse("/register example.com alice").is_err());
        assert!(parse("/register example.com").is_err());
        assert!(parse("/register").is_err());
    }

    #[test]
    fn test_parse_login() {
        let cmd = parse("/login example.com alice pass1234").unwrap();
        let Command::Login {
            server,
            username,
            password,
        } = cmd
        else {
            panic!("expected Login variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "alice");
        assert_eq!(password, "pass1234");
    }

    #[test]
    fn test_parse_login_missing_args() {
        assert!(parse("/login example.com alice").is_err());
        assert!(parse("/login example.com").is_err());
        assert!(parse("/login").is_err());
    }

    #[test]
    fn test_parse_passwd() {
        let cmd = parse("/passwd newpass456").unwrap();
        let Command::Passwd { new_password } = cmd else {
            panic!("expected Passwd variant");
        };
        assert_eq!(new_password, "newpass456");
    }

    #[test]
    fn test_parse_passwd_new_password_with_spaces() {
        let cmd = parse("/passwd new pass with spaces").unwrap();
        let Command::Passwd { new_password } = cmd else {
            panic!("expected Passwd variant");
        };
        assert_eq!(new_password, "new pass with spaces");
    }

    #[test]
    fn test_parse_passwd_missing_args() {
        assert!(parse("/passwd").is_err());
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
    fn test_parse_list() {
        let cmd = parse("/list").unwrap();
        assert!(matches!(cmd, Command::List));
    }

    #[test]
    fn test_parse_who() {
        let cmd = parse("/who").unwrap();
        assert!(matches!(cmd, Command::Who));
    }

    // Messaging

    #[test]
    fn test_parse_plain_message() {
        let cmd = parse("hello").unwrap();
        let Command::Message { text } = cmd else {
            panic!("expected Message variant");
        };
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_parse_msg() {
        let cmd = parse("/msg room hello world").unwrap();
        let Command::Msg { room, text } = cmd else {
            panic!("expected Msg variant");
        };
        assert_eq!(room, "room");
        assert_eq!(text, "hello world");
    }

    #[test]
    fn test_parse_msg_missing_args() {
        assert!(parse("/msg room").is_err());
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
    fn test_parse_whois() {
        let cmd = parse("/whois").unwrap();
        assert!(matches!(cmd, Command::Whois));
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
    fn test_parse_nick() {
        let cmd = parse("/nick Alice Smith").unwrap();
        let Command::Nick { alias } = cmd else {
            panic!("expected Nick variant");
        };
        assert_eq!(alias, "Alice Smith");
    }

    #[test]
    fn test_parse_nick_missing_args() {
        assert!(parse("/nick").is_err());
    }

    #[test]
    fn test_parse_password_with_spaces() {
        let cmd = parse("/login example.com user pass word here").unwrap();
        let Command::Login {
            server,
            username,
            password,
        } = cmd
        else {
            panic!("expected Login variant");
        };
        assert_eq!(server, "example.com");
        assert_eq!(username, "user");
        assert_eq!(password, "pass word here");
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
        let cmd = parse("/accept 42").unwrap();
        let Command::Accept { invite_id } = cmd else {
            panic!("expected Accept variant");
        };
        assert_eq!(invite_id, Some(42));
    }

    #[test]
    fn test_parse_accept_invalid_id() {
        assert!(parse("/accept abc").is_err());
    }

    #[test]
    fn test_parse_decline() {
        let cmd = parse("/decline 7").unwrap();
        let Command::Decline { invite_id } = cmd else {
            panic!("expected Decline variant");
        };
        assert_eq!(invite_id, 7);
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
