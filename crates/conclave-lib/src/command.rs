use crate::error::{Error, Result};

/// Parsed command from user input.
pub enum Command {
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
    Create {
        name: String,
        members: Vec<String>,
    },
    /// No args: accept pending welcomes. With arg: switch to room.
    Join {
        target: Option<String>,
    },
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
    /// Leave the room (MLS removal). IRC: /part
    Part,
    /// Switch away from the active room without leaving. IRC: /close
    Close,
    Rotate,
    Reset,
    Info,
    /// List rooms. IRC: /list
    List,
    /// List members of the active room. IRC: /who, /names
    Who,
    Msg {
        room: String,
        text: String,
    },
    Unread,
    Logout,
    /// Change display name. IRC: /nick
    Nick {
        alias: String,
    },
    /// Set active room's display alias. IRC: /topic
    Topic {
        topic: String,
    },
    /// Show current user info. IRC: /whois
    Whois,
    Help,
    Quit,
    Message {
        text: String,
    },
}

/// Parse user input into a Command.
pub fn parse(input: &str) -> Result<Command> {
    if !input.starts_with('/') {
        return Ok(Command::Message {
            text: input.to_string(),
        });
    }

    let parts: Vec<&str> = input.splitn(3, ' ').collect();
    let cmd = parts[0];

    match cmd {
        "/register" => {
            // /register needs 4 parts: cmd server username password
            let parts: Vec<&str> = input.splitn(4, ' ').collect();
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
        "/login" => {
            // /login needs 4 parts: cmd server username password
            let parts: Vec<&str> = input.splitn(4, ' ').collect();
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
        "/create" => {
            if parts.len() < 3 {
                return Err(Error::Other(
                    "Usage: /create <name> <member1,member2,...>".into(),
                ));
            }
            let members = parts[2].split(',').map(|s| s.trim().to_string()).collect();
            Ok(Command::Create {
                name: parts[1].to_string(),
                members,
            })
        }
        "/join" => {
            let target = parts.get(1).map(|s| s.to_string());
            Ok(Command::Join { target })
        }
        "/invite" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /invite <member1,member2,...>".into()));
            }
            let members = parts[1].split(',').map(|s| s.trim().to_string()).collect();
            Ok(Command::Invite { members })
        }
        "/kick" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /kick <username>".into()));
            }
            Ok(Command::Kick {
                username: parts[1].to_string(),
            })
        }
        "/promote" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /promote <username>".into()));
            }
            Ok(Command::Promote {
                username: parts[1].to_string(),
            })
        }
        "/demote" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /demote <username>".into()));
            }
            Ok(Command::Demote {
                username: parts[1].to_string(),
            })
        }
        "/admins" => Ok(Command::Admins),
        "/part" => Ok(Command::Part),
        "/close" => Ok(Command::Close),
        "/rotate" => Ok(Command::Rotate),
        "/reset" => Ok(Command::Reset),
        "/info" => Ok(Command::Info),
        "/list" => Ok(Command::List),
        "/who" => Ok(Command::Who),
        "/msg" => {
            if parts.len() < 3 {
                return Err(Error::Other("Usage: /msg <room> <message>".into()));
            }
            Ok(Command::Msg {
                room: parts[1].to_string(),
                text: parts[2].to_string(),
            })
        }
        "/nick" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /nick <alias>".into()));
            }
            let alias = input["/nick ".len()..].to_string();
            Ok(Command::Nick { alias })
        }
        "/topic" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /topic <text>".into()));
            }
            let topic = input["/topic ".len()..].to_string();
            Ok(Command::Topic { topic })
        }
        "/unread" => Ok(Command::Unread),
        "/logout" => Ok(Command::Logout),
        "/whois" => Ok(Command::Whois),
        "/help" | "/h" => Ok(Command::Help),
        "/quit" | "/exit" | "/q" => Ok(Command::Quit),
        _ => Err(Error::Other(format!(
            "Unknown command: {cmd}. Type /help for available commands."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_register() {
        let cmd = parse("/register example.com alice pass1234").unwrap();
        if let Command::Register {
            server,
            username,
            password,
        } = cmd
        {
            assert_eq!(server, "example.com");
            assert_eq!(username, "alice");
            assert_eq!(password, "pass1234");
        } else {
            panic!("wrong variant");
        }
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
        if let Command::Login {
            server,
            username,
            password,
        } = cmd
        {
            assert_eq!(server, "example.com");
            assert_eq!(username, "alice");
            assert_eq!(password, "pass1234");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_login_missing_args() {
        assert!(parse("/login example.com alice").is_err());
        assert!(parse("/login example.com").is_err());
        assert!(parse("/login").is_err());
    }

    #[test]
    fn test_parse_create() {
        let cmd = parse("/create room alice,bob").unwrap();
        if let Command::Create { name, members } = cmd {
            assert_eq!(name, "room");
            assert_eq!(members, vec!["alice", "bob"]);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_create_missing_args() {
        assert!(parse("/create room").is_err());
    }

    #[test]
    fn test_parse_join_no_arg() {
        let cmd = parse("/join").unwrap();
        if let Command::Join { target } = cmd {
            assert!(target.is_none());
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_join_with_target() {
        let cmd = parse("/join myroom").unwrap();
        if let Command::Join { target } = cmd {
            assert_eq!(target, Some("myroom".to_string()));
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_invite() {
        let cmd = parse("/invite alice,bob").unwrap();
        if let Command::Invite { members } = cmd {
            assert_eq!(members, vec!["alice", "bob"]);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_invite_missing_args() {
        assert!(parse("/invite").is_err());
    }

    #[test]
    fn test_parse_kick() {
        let cmd = parse("/kick alice").unwrap();
        if let Command::Kick { username } = cmd {
            assert_eq!(username, "alice");
        } else {
            panic!("wrong variant");
        }
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

    #[test]
    fn test_parse_msg() {
        let cmd = parse("/msg room hello world").unwrap();
        if let Command::Msg { room, text } = cmd {
            assert_eq!(room, "room");
            assert_eq!(text, "hello world");
        } else {
            panic!("wrong variant");
        }
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
    fn test_parse_plain_message() {
        let cmd = parse("hello").unwrap();
        if let Command::Message { text } = cmd {
            assert_eq!(text, "hello");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_nick() {
        let cmd = parse("/nick Alice Smith").unwrap();
        if let Command::Nick { alias } = cmd {
            assert_eq!(alias, "Alice Smith");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_nick_missing_args() {
        assert!(parse("/nick").is_err());
    }

    #[test]
    fn test_parse_topic() {
        let cmd = parse("/topic Dev Team Chat").unwrap();
        if let Command::Topic { topic } = cmd {
            assert_eq!(topic, "Dev Team Chat");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_topic_missing_args() {
        assert!(parse("/topic").is_err());
    }

    #[test]
    fn test_parse_unknown_command() {
        assert!(parse("/xyz").is_err());
    }

    #[test]
    fn test_parse_password_with_spaces() {
        let cmd = parse("/login example.com user pass word here").unwrap();
        if let Command::Login {
            server,
            username,
            password,
        } = cmd
        {
            assert_eq!(server, "example.com");
            assert_eq!(username, "user");
            assert_eq!(password, "pass word here");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_admins() {
        let cmd = parse("/admins").unwrap();
        assert!(matches!(cmd, Command::Admins));
    }

    #[test]
    fn test_parse_promote() {
        let cmd = parse("/promote alice").unwrap();
        if let Command::Promote { username } = cmd {
            assert_eq!(username, "alice");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_promote_missing_username() {
        assert!(parse("/promote").is_err());
    }

    #[test]
    fn test_parse_demote() {
        let cmd = parse("/demote bob").unwrap();
        if let Command::Demote { username } = cmd {
            assert_eq!(username, "bob");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_parse_demote_missing_username() {
        assert!(parse("/demote").is_err());
    }
}
