//! Snapshots of the model-facing tool definitions, system prompt, and attachment notes in
//! `hatoba_ai::tools`.

use hatoba_ai::tools::{
    LocalPrompt, MemoryEntry, PermissionMode, PromptContext, PromptModel, PromptTools, ToolPlace,
    ToolSet, builtin_tools, deferred_mcp_tools, file_revert_block, host_change_block,
    interrupted_block, memory_block, model_change_block, permission_mode_block, shell_change_block,
    system_prompt, terminal_change_block, terminal_state_block,
};
use insta::{assert_json_snapshot, assert_snapshot};

#[test]
fn builtin_tools_snapshots() {
    let no_terminal_none = ToolSet::default();
    assert_json_snapshot!(
        "builtin_tools_no_terminal_none",
        builtin_tools(&no_terminal_none)
    );

    let no_terminal_all = ToolSet {
        terminal: false,
        web_search: true,
        read_skill: true,
        memory: true,
        place: ToolPlace::Ssh,
    };
    assert_json_snapshot!(
        "builtin_tools_no_terminal_all",
        builtin_tools(&no_terminal_all)
    );

    // When `terminal` is false, `place` has no effect on the returned definitions.
    for place in [
        ToolPlace::Ssh,
        ToolPlace::Local,
        ToolPlace::LocalWindows,
        ToolPlace::Telnet,
    ] {
        assert_eq!(
            builtin_tools(&ToolSet {
                place,
                ..no_terminal_none
            }),
            builtin_tools(&no_terminal_none)
        );
        assert_eq!(
            builtin_tools(&ToolSet {
                place,
                ..no_terminal_all
            }),
            builtin_tools(&no_terminal_all)
        );
    }

    let places = [
        ("ssh", ToolPlace::Ssh),
        ("local", ToolPlace::Local),
        ("local_windows", ToolPlace::LocalWindows),
        ("telnet", ToolPlace::Telnet),
    ];
    for (label, place) in places {
        let terminal_only = ToolSet {
            terminal: true,
            web_search: false,
            read_skill: false,
            memory: false,
            place,
        };
        assert_json_snapshot!(
            format!("builtin_tools_{label}_terminal_only"),
            builtin_tools(&terminal_only)
        );

        let all = ToolSet {
            terminal: true,
            web_search: true,
            read_skill: true,
            memory: true,
            place,
        };
        assert_json_snapshot!(format!("builtin_tools_{label}_all"), builtin_tools(&all));
    }

    assert_json_snapshot!("deferred_mcp_tools", deferred_mcp_tools());
}

fn base_prompt_context<'a>(skills: &'a [(String, String)]) -> PromptContext<'a> {
    PromptContext {
        model: PromptModel {
            id: "claude-opus-5-5",
            name: "Claude Opus 5.5",
            provider: "Anthropic",
        },
        host_name: Some("prod-db"),
        host_user: Some("ops"),
        server_id: Some("SSH-2.0-OpenSSH_9.6p1"),
        local: None,
        telnet: false,
        date: "2026-10-09",
        instructions: "Answer in English.",
        host_notes: "PostgreSQL 16 primary.",
        skills,
        memory: false,
        tools: PromptTools::Terminal,
    }
}

#[test]
fn system_prompt_snapshots() {
    let skills = vec![
        ("hatoba".to_owned(), "How Hatoba works.".to_owned()),
        ("nginx".to_owned(), "Nginx checks.".to_owned()),
    ];
    let base = base_prompt_context(&skills);
    let local_unix = LocalPrompt {
        device: "macbook",
        os: "macOS",
        shell: "zsh",
        place: ToolPlace::Local,
        wsl: false,
    };
    let local_wsl = LocalPrompt {
        device: "workstation",
        os: "Windows",
        shell: "Ubuntu",
        place: ToolPlace::Local,
        wsl: true,
    };
    let local_windows = LocalPrompt {
        device: "workstation",
        os: "Windows",
        shell: "PowerShell",
        place: ToolPlace::LocalWindows,
        wsl: false,
    };

    for (label, tools) in [
        ("none", PromptTools::None),
        ("side_question", PromptTools::SideQuestion),
    ] {
        let prompt = system_prompt(&PromptContext { tools, ..base });
        assert_snapshot!(format!("system_prompt_{label}"), prompt);
        // local, telnet and memory flags do not alter None or SideQuestion prompts.
        for memory in [false, true] {
            for telnet in [false, true] {
                for local in [None, Some(local_unix), Some(local_windows)] {
                    assert_eq!(
                        system_prompt(&PromptContext {
                            tools,
                            memory,
                            telnet,
                            local,
                            ..base
                        }),
                        prompt
                    );
                }
            }
        }
    }

    for (mem_label, memory) in [("memory_off", false), ("memory_on", true)] {
        let no_term = system_prompt(&PromptContext {
            tools: PromptTools::NoTerminal,
            memory,
            ..base
        });
        assert_snapshot!(format!("system_prompt_no_terminal_{mem_label}"), no_term);
        // local and telnet flags have no effect when tools == NoTerminal.
        assert_eq!(
            system_prompt(&PromptContext {
                tools: PromptTools::NoTerminal,
                memory,
                local: Some(local_unix),
                telnet: true,
                ..base
            }),
            no_term
        );

        let ssh = system_prompt(&PromptContext {
            tools: PromptTools::Terminal,
            memory,
            ..base
        });
        assert_snapshot!(format!("system_prompt_terminal_ssh_{mem_label}"), ssh);

        let telnet = system_prompt(&PromptContext {
            tools: PromptTools::Terminal,
            telnet: true,
            memory,
            ..base
        });
        assert_snapshot!(format!("system_prompt_terminal_telnet_{mem_label}"), telnet);

        for (local_label, local) in [
            ("local_unix", local_unix),
            ("local_wsl", local_wsl),
            ("local_windows", local_windows),
        ] {
            let p = system_prompt(&PromptContext {
                tools: PromptTools::Terminal,
                local: Some(local),
                host_name: Some(local.device),
                host_user: Some("alice"),
                server_id: None,
                host_notes: "",
                memory,
                ..base
            });
            assert_snapshot!(
                format!("system_prompt_terminal_{local_label}_{mem_label}"),
                p
            );
        }
    }
}

#[test]
fn note_block_snapshots() {
    assert_snapshot!("block_interrupted", interrupted_block());
    assert_snapshot!(
        "block_host_change_named",
        host_change_block("staging-web", "prod-db")
    );
    assert_snapshot!(
        "block_host_change_empty_from",
        host_change_block("", "prod-db")
    );
    assert_snapshot!(
        "block_terminal_change_attached",
        terminal_change_block(true)
    );
    assert_snapshot!(
        "block_terminal_change_detached",
        terminal_change_block(false)
    );
    assert_snapshot!(
        "block_shell_change",
        shell_change_block("Ubuntu", "PowerShell")
    );
    assert_snapshot!(
        "block_model_change",
        model_change_block(
            "Claude Sonnet 5.5 (claude-sonnet-5-5)",
            "Claude Opus 5.5 (claude-opus-5-5)"
        )
    );
    assert_snapshot!(
        "block_file_revert_edited",
        file_revert_block("/etc/nginx/nginx.conf", "toolu_01", "edit_file", false)
    );
    assert_snapshot!(
        "block_file_revert_created",
        file_revert_block("/etc/nginx/new.conf", "toolu_02", "write_file", true)
    );
    assert_snapshot!(
        "block_permission_mode_manual",
        permission_mode_block(PermissionMode::Manual)
    );
    assert_snapshot!(
        "block_permission_mode_auto",
        permission_mode_block(PermissionMode::Auto)
    );
    assert_snapshot!(
        "block_permission_mode_bypass",
        permission_mode_block(PermissionMode::Bypass)
    );
    assert_snapshot!(
        "block_memory",
        memory_block(
            Some("prod-db"),
            &[
                MemoryEntry {
                    name: "Replies language",
                    updated: "",
                    host: None,
                    content: "Answer in English.",
                },
                MemoryEntry {
                    name: "sudo",
                    updated: "2026-10-03",
                    host: Some("prod-db"),
                    content: "Uses doas.",
                },
            ]
        )
    );
    assert_snapshot!(
        "block_terminal_state_normal",
        terminal_state_block("prod-api", false, "\n\n$ make\nerror: no rule\n\n$   \n\n")
    );
    assert_snapshot!(
        "block_terminal_state_alternate",
        terminal_state_block("prod-api", true, "  ~\n\"x.conf\" 3L")
    );
}
