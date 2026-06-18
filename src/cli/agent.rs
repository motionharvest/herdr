use crate::api::schema::{
    AgentReadParams, AgentRenameParams, AgentSendParams, AgentStartParams, AgentStatus,
    AgentTarget, EmptyParams, Method, ReadFormat, ReadSource, Request, Subscription,
};

pub(super) fn run_agent_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_agent_help();
        return Ok(2);
    };

    match subcommand {
        "list" => agent_list(&args[1..]),
        "get" => agent_get(&args[1..]),
        "read" => agent_read(&args[1..]),
        "send" => agent_send(&args[1..]),
        "rename" => agent_rename(&args[1..]),
        "focus" => agent_focus(&args[1..]),
        "wait" => agent_wait(&args[1..]),
        "attach" => agent_attach(&args[1..]),
        "start" => agent_start(&args[1..]),
        "help" | "--help" | "-h" => {
            print_agent_help();
            Ok(0)
        }
        _ => {
            print_agent_help();
            Ok(2)
        }
    }
}

fn agent_start(args: &[String]) -> std::io::Result<i32> {
    let Some(name) = args.first() else {
        eprintln!("usage: herdr agent start <name> [--cwd PATH] [--workspace ID] [--tab ID] [--split right|down] [--focus|--no-focus] -- <argv...>");
        return Ok(2);
    };

    let Some(separator) = args.iter().position(|arg| arg == "--") else {
        eprintln!("usage: herdr agent start <name> [--cwd PATH] [--workspace ID] [--tab ID] [--split right|down] [--focus|--no-focus] -- <argv...>");
        return Ok(2);
    };
    if separator == args.len() - 1 {
        eprintln!("agent start requires argv after --");
        return Ok(2);
    }

    let mut cwd = None;
    let mut workspace_id = None;
    let mut tab_id = None;
    let mut split = None;
    let mut focus = false;

    let mut index = 1;
    while index < separator {
        match args[index].as_str() {
            "--cwd" => {
                let Some(value) = args.get(index + 1).filter(|_| index + 1 < separator) else {
                    eprintln!("missing value for --cwd");
                    return Ok(2);
                };
                cwd = Some(value.clone());
                index += 2;
            }
            "--workspace" => {
                let Some(value) = args.get(index + 1).filter(|_| index + 1 < separator) else {
                    eprintln!("missing value for --workspace");
                    return Ok(2);
                };
                workspace_id = Some(super::normalize_workspace_id(value));
                index += 2;
            }
            "--tab" => {
                let Some(value) = args.get(index + 1).filter(|_| index + 1 < separator) else {
                    eprintln!("missing value for --tab");
                    return Ok(2);
                };
                tab_id = Some(super::normalize_tab_id(value));
                index += 2;
            }
            "--split" => {
                let Some(value) = args.get(index + 1).filter(|_| index + 1 < separator) else {
                    eprintln!("missing value for --split");
                    return Ok(2);
                };
                split = Some(super::parse_split_direction(value)?);
                index += 2;
            }
            "--focus" => {
                focus = true;
                index += 1;
            }
            "--no-focus" => {
                focus = false;
                index += 1;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:start".into(),
        method: Method::AgentStart(AgentStartParams {
            name: name.clone(),
            cwd,
            workspace_id,
            tab_id,
            split,
            focus,
            argv: args[separator + 1..].to_vec(),
        }),
    })?)
}

fn agent_list(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        eprintln!("usage: herdr agent list");
        return Ok(2);
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:list".into(),
        method: Method::AgentList(EmptyParams::default()),
    })?)
}

fn agent_get(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = args.first() else {
        eprintln!("usage: herdr agent get <target>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr agent get <target>");
        return Ok(2);
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:get".into(),
        method: Method::AgentGet(AgentTarget {
            target: target.clone(),
        }),
    })?)
}

fn agent_focus(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = args.first() else {
        eprintln!("usage: herdr agent focus <target>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr agent focus <target>");
        return Ok(2);
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:focus".into(),
        method: Method::AgentFocus(AgentTarget {
            target: target.clone(),
        }),
    })?)
}

fn agent_attach(args: &[String]) -> std::io::Result<i32> {
    let (target, takeover) =
        match super::parse_attach_target(args, "usage: herdr agent attach <target> [--takeover]") {
            Ok(parsed) => parsed,
            Err(code) => return Ok(code),
        };

    let response = resolve_agent_target(&target, "cli:agent:attach:resolve")?;
    if response.get("error").is_some() {
        eprintln!("{}", serde_json::to_string(&response).unwrap());
        return Ok(1);
    }
    let Some(terminal_id) = response["result"]["agent"]["terminal_id"].as_str() else {
        eprintln!("agent attach failed: response did not include terminal_id");
        return Ok(1);
    };
    crate::client::run_terminal_attach(terminal_id.to_owned(), takeover)?;
    Ok(0)
}

fn agent_wait(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = args.first() else {
        eprintln!("usage: herdr agent wait <target> --status <idle|working|blocked|unknown> [--timeout MS]");
        return Ok(2);
    };

    let mut timeout_ms = None;
    let mut desired_status = None;

    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--status" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --status");
                    return Ok(2);
                };
                desired_status = Some(parse_agent_wait_status(value)?);
                index += 2;
            }
            "--timeout" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --timeout");
                    return Ok(2);
                };
                timeout_ms = Some(super::parse_u64_flag("--timeout", value)?);
                index += 2;
            }
            "help" | "--help" | "-h" => {
                eprintln!("usage: herdr agent wait <target> --status <idle|working|blocked|unknown> [--timeout MS]");
                return Ok(0);
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    let Some(agent_status) = desired_status else {
        eprintln!("missing required --status");
        return Ok(2);
    };

    let response = resolve_agent_target(target, "cli:agent:wait:resolve")?;
    if response.get("error").is_some() {
        eprintln!("{}", serde_json::to_string(&response).unwrap());
        return Ok(1);
    }
    if response["result"]["agent"]["agent_status"]
        .as_str()
        .is_some_and(|current| agent_wait_status_satisfied(agent_status, current))
    {
        println!("{}", serde_json::to_string(&response).unwrap());
        return Ok(0);
    }

    let Some(pane_id) = response["result"]["agent"]["pane_id"].as_str() else {
        eprintln!("agent wait failed: response did not include pane_id");
        return Ok(1);
    };

    let subscriptions = if agent_status == AgentStatus::Idle {
        vec![
            Subscription::PaneAgentStatusChanged {
                pane_id: pane_id.to_owned(),
                agent_status: Some(AgentStatus::Idle),
            },
            Subscription::PaneAgentStatusChanged {
                pane_id: pane_id.to_owned(),
                agent_status: Some(AgentStatus::Done),
            },
        ]
    } else {
        vec![Subscription::PaneAgentStatusChanged {
            pane_id: pane_id.to_owned(),
            agent_status: Some(agent_status),
        }]
    };

    super::wait_for_agent_change(
        Request {
            id: "cli:agent:wait".into(),
            method: Method::EventsSubscribe(crate::api::schema::EventsSubscribeParams {
                subscriptions,
            }),
        },
        timeout_ms,
        "timed out waiting for agent status change",
    )
}

fn resolve_agent_target(target: &str, request_id: &str) -> std::io::Result<serde_json::Value> {
    super::send_request(&Request {
        id: request_id.into(),
        method: Method::AgentGet(AgentTarget {
            target: target.to_owned(),
        }),
    })
}

fn agent_rename(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = args.first() else {
        eprintln!("usage: herdr agent rename <target> <name>|--clear");
        return Ok(2);
    };
    if args.len() < 2 {
        eprintln!("usage: herdr agent rename <target> <name>|--clear");
        return Ok(2);
    }
    let name = if args.len() == 2 && args[1] == "--clear" {
        None
    } else {
        Some(args[1..].join(" "))
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:rename".into(),
        method: Method::AgentRename(AgentRenameParams {
            target: target.clone(),
            name,
        }),
    })?)
}

fn agent_send(args: &[String]) -> std::io::Result<i32> {
    if args.len() < 2 {
        eprintln!("usage: herdr agent send <target> <text>");
        return Ok(2);
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:send".into(),
        method: Method::AgentSend(AgentSendParams {
            target: args[0].clone(),
            text: args[1..].join(" "),
        }),
    })?)
}

fn agent_read(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = args.first() else {
        eprintln!("usage: herdr agent read <target> [--source visible|recent|recent-unwrapped] [--lines N] [--format text|ansi] [--ansi]");
        return Ok(2);
    };

    let mut source = ReadSource::Recent;
    let mut lines = None;
    let mut format = ReadFormat::Text;
    let mut strip_ansi = true;

    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --source");
                    return Ok(2);
                };
                source = super::parse_read_source(value)?;
                index += 2;
            }
            "--lines" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --lines");
                    return Ok(2);
                };
                lines = Some(super::parse_u32_flag("--lines", value)?);
                index += 2;
            }
            "--format" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --format");
                    return Ok(2);
                };
                format = super::parse_read_format(value)?;
                strip_ansi = !matches!(format, ReadFormat::Ansi);
                index += 2;
            }
            "--ansi" => {
                format = ReadFormat::Ansi;
                strip_ansi = false;
                index += 1;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:agent:read".into(),
        method: Method::AgentRead(AgentReadParams {
            target: target.clone(),
            source,
            lines,
            format,
            strip_ansi,
        }),
    })?)
}

fn agent_wait_status_satisfied(desired: AgentStatus, current: &str) -> bool {
    match desired {
        AgentStatus::Idle => matches!(current, "idle" | "done"),
        AgentStatus::Working => current == "working",
        AgentStatus::Blocked => current == "blocked",
        AgentStatus::Unknown => current == "unknown",
        AgentStatus::Done => false,
    }
}

fn parse_agent_wait_status(value: &str) -> std::io::Result<AgentStatus> {
    match value {
        "idle" => Ok(AgentStatus::Idle),
        "working" => Ok(AgentStatus::Working),
        "blocked" => Ok(AgentStatus::Blocked),
        "unknown" => Ok(AgentStatus::Unknown),
        "done" => Err(std::io::Error::other(
            "done is a UI attention state; use idle for CLI agent completion waits",
        )),
        _ => Err(std::io::Error::other(format!(
            "invalid agent status: {value} (expected idle, working, blocked, or unknown)"
        ))),
    }
}

fn print_agent_help() {
    eprintln!("herdr agent commands:");
    eprintln!("  herdr agent list");
    eprintln!("  herdr agent get <target>");
    eprintln!("  herdr agent read <target> [--source visible|recent|recent-unwrapped] [--lines N] [--format text|ansi] [--ansi]");
    eprintln!("  herdr agent send <target> <text>");
    eprintln!("  herdr agent rename <target> <name>|--clear");
    eprintln!("  herdr agent focus <target>");
    eprintln!("  herdr agent wait <target> --status <idle|working|blocked|unknown> [--timeout MS]");
    eprintln!("  herdr agent attach <target> [--takeover]");
    eprintln!("  herdr agent start <name> [--cwd PATH] [--workspace ID] [--tab ID] [--split right|down] [--focus|--no-focus] -- <argv...>");
    eprintln!("  targets accept terminal ids, unique agent names, detected/reported agent labels, and legacy pane ids");
    eprintln!(
        "  agent send writes literal text; use pane run when you want command text plus Enter"
    );
}
