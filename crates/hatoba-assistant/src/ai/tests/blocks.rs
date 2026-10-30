//! The blocks a message starts with (spec §13.3): titles, the cases the panel shares, and what
//! commands and mentions attach, as stored and as the judge reads it.

use super::*;

#[test]
fn titles_skip_the_attachment_blocks_a_message_starts_with() {
    // AI-23: the first line of the first message, cut to 60 characters.
    assert_eq!(title_of("  check the disk\nplease"), "check the disk");
    assert_eq!(title_of(&"长".repeat(80)).chars().count(), 60);

    // AI-10: the title comes from the typed text, not from the block (spec §13.3).
    let selection = "<terminal_selection host=\"prod-api\" lines=\"2\">\n$ make\nerror: boom\n\
                     </terminal_selection>\n\n  \nWhy does make fail?\nmore";
    assert_eq!(title_of(selection), "Why does make fail?");
    // Escaped attribute values, a truncated selection, and closing tags escaped in the body.
    let escaped = "<terminal_selection host=\"web &quot;a&quot; &lt;b&gt;\" lines=\"3\" truncated=\"true\">\n\
                   echo \"<\\/terminal_selection>\"\n<\\\\/terminal_selection> and <\\/TERMINAL_SELECTION>\n\
                   </terminal_selection>\n\nexplain this";
    assert_eq!(title_of(escaped), "explain this");
    // A closing tag that other text follows on its line does not end the block.
    let inner = "<terminal_selection host=\"x\" lines=\"3\">\na\n</terminal_selection> tail\nb\n\
                 </terminal_selection>\nwhat now";
    assert_eq!(title_of(inner), "what now");
    let diagnostics = "<connection_diagnostics host=\"db-1\">\nConnection refused (os error 111)\n\
                       </connection_diagnostics>\n\nWhy can't I connect?";
    assert_eq!(title_of(diagnostics), "Why can't I connect?");
    // Both kinds, one after the other.
    let both = "<connection_diagnostics host=\"db-1\">\nrefused\n</connection_diagnostics>\n\n\
                <terminal_selection host=\"db-1\" lines=\"1\">\n$ ssh db-1\n</terminal_selection>\n\nhelp";
    assert_eq!(title_of(both), "help");

    // Nothing typed: the block's name.
    assert_eq!(
        title_of("<terminal_selection host=\"x\" lines=\"1\">\nls\n</terminal_selection>"),
        "Terminal selection"
    );
    assert_eq!(
        title_of("<connection_diagnostics host=\"x\">\nboom\n</connection_diagnostics>\n\n \n"),
        "Connection diagnostics"
    );

    // AI-35: pasted text and files, several of each, after the diagnostics and the selection.
    let many = "<connection_diagnostics host=\"db-1\">\nrefused\n</connection_diagnostics>\n\n\
                <terminal_selection host=\"db-1\" lines=\"1\">\n$ ssh db-1\n</terminal_selection>\n\n\
                <pasted_text lines=\"2\">\na\n<\\/pasted_text>\n</pasted_text>\n\n\
                <file name=\"nginx.conf\" lines=\"1\">\nserver {}\n</file>\n\n\
                <pasted_text lines=\"1\">\nb\n</pasted_text>\n\n\
                <file name=\"a.log\" lines=\"1\">\n</file> inside\n</file>\n\n\
                What is wrong here?";
    assert_eq!(title_of(many), "What is wrong here?");
    assert_eq!(
        title_of("<pasted_text lines=\"3\">\na\nb\nc\n</pasted_text>"),
        "Pasted text"
    );
    // A file's name, with its attribute escapes read back.
    assert_eq!(
        title_of(
            "<file name=\"a &quot;b&quot; &lt;c&gt; &amp; d.txt\" lines=\"1\">\nx\n</file>\n\n"
        ),
        "a \"b\" <c> & d.txt"
    );
    assert_eq!(
        title_of(&format!(
            "<file name=\"{}.txt\" lines=\"1\">\nx\n</file>",
            "长".repeat(70)
        ))
        .chars()
        .count(),
        60
    );

    // AI-05, AI-09: Hatoba's notes come first and never title the conversation.
    let notes = "<interrupted>\nStopped.\n</interrupted>\n\n\
                 <host_change from=\"staging-web\" to=\"prod-db\">\nThe conversation moved.\n\
                 </host_change>\n\n<model_change from=\"A (a)\" to=\"B (b)\">\nOther model.\n\
                 </model_change>\n\n";
    assert_eq!(
        title_of(&format!(
            "{notes}<file name=\"a.log\" lines=\"1\">\nx\n</file>\n\nWhat now?"
        )),
        "What now?"
    );
    assert_eq!(
        title_of(&format!(
            "{notes}<pasted_text lines=\"1\">\nx\n</pasted_text>"
        )),
        "Pasted text"
    );
    assert_eq!(title_of(notes), "");
    // A note whose attributes are not the kind's own, a note after an attachment, or a second
    // note of a kind, is typed text.
    let odd = "<host_change to=\"b\" from=\"a\">\nx\n</host_change>\n\nq";
    assert_eq!(title_of(odd), "<host_change to=\"b\" from=\"a\">");
    let late = "<pasted_text lines=\"1\">\nx\n</pasted_text>\n\n\
                <host_change from=\"a\" to=\"b\">\nx\n</host_change>\n\nq";
    assert_eq!(title_of(late), "<host_change from=\"a\" to=\"b\">");
    let twice = "<model_change from=\"a\" to=\"b\">\nx\n</model_change>\n\n\
                 <model_change from=\"b\" to=\"c\">\nx\n</model_change>\n\nq";
    assert_eq!(title_of(twice), "<model_change from=\"b\" to=\"c\">");

    // Anything else is typed text.
    for text in [
        "see <terminal_selection host=\"x\" lines=\"1\">\nx\n</terminal_selection>",
        "<terminal_selection host=\"x\" lines=\"1\">\nno end",
        "<terminal_selection host=\"x\">\nno lines\n</terminal_selection>\n\nq",
        "<terminal_selection host=\"x\" lines=\"one\">\nx\n</terminal_selection>\n\nq",
        "<terminal_selection host=\"x\" lines=\"1\" truncated=\"yes\">\nx\n</terminal_selection>\n\nq",
        "<connection_diagnostics host=\"x\" lines=\"1\">\nx\n</connection_diagnostics>\n\nq",
        "<other_block host=\"x\">\nx\n</other_block>\n\nq",
        "<pasted_text>\nx\n</pasted_text>\n\nq",
        "<pasted_text lines=\"x\">\nx\n</pasted_text>\n\nq",
        "<file lines=\"1\" name=\"a.txt\">\nx\n</file>\n\nq",
        "<file name=\"a.txt\">\nx\n</file>\n\nq",
        "<files name=\"a.txt\" lines=\"1\">\nx\n</files>\n\nq",
        "<terminal_selection host=\"x\" lines=\"1\">x\n</terminal_selection>\n\nq",
    ] {
        assert_eq!(
            title_of(text),
            text.lines().next().unwrap().trim(),
            "{text:?}"
        );
    }

    // A selection block typed after an attached selection is typed text, as the panel reads it.
    let typed_block = "<terminal_selection host=\"x\" lines=\"1\">\nls\n</terminal_selection>\n\n\
                       <terminal_selection host=\"example\" lines=\"1\">\nWhy?\n</terminal_selection>";
    assert_eq!(
        title_of(typed_block),
        "<terminal_selection host=\"example\" lines=\"1\">"
    );
}

#[test]
fn leading_blocks_read_the_shared_cases_as_the_panel_does() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        text: String,
        blocks: usize,
        typed: String,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../../../apps/desktop/src/features/ai/blocks.fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let (blocks, typed) = leading_blocks(&case.text);
        assert_eq!(
            (blocks.len(), typed),
            (case.blocks, case.typed.as_str()),
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn a_message_with_a_selection_keeps_the_block_and_is_titled_after_the_typed_text() {
    let f = Fixture::new(vec![answer("It failed to compile.")]).await;
    let text = "<terminal_selection host=\"prod-api\" lines=\"1\">\nerror: boom\n\
                </terminal_selection>\n\nWhy?";
    let (started, sink) = f.send(None, text).await;
    assert_eq!(started.conversation.title, "Why?");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    // The block is meant for the model: the request carries the message as stored.
    let requests = f.requests().await;
    let user = messages(&requests[0])
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap();
    assert_eq!(user["content"], manual(text));
}

#[tokio::test]
async fn a_message_with_attachments_as_large_as_the_panel_allows_is_stored_whole() {
    // AI-35: the panel caps a message's attachments at 512 KB; the entry is split into parts of at
    // most 40 KB (§13.7), and nothing on the way cuts it.
    let f = Fixture::new(vec![answer("Read it all.")]).await;
    let file =
        "2026-10-09 09:41:12 nginx[812]: upstream \"timed out\" <\\/file> 日本語\n".repeat(3_500);
    let paste = "p".repeat(262_000);
    let text = format!(
        "<pasted_text lines=\"1\">\n{paste}\n</pasted_text>\n\n\
         <file name=\"error.log\" lines=\"3500\">\n{file}\n</file>\n\nWhat failed?"
    );
    assert!(text.len() > 500 * 1024, "{} bytes", text.len());
    let (started, sink) = f.send(None, &text).await;
    assert_eq!(started.conversation.title, "What failed?");
    assert_eq!(sink.ended().await, AiTurnEndReason::Completed);
    let detail = f.detail(&started.conversation.id);
    let stored = detail
        .entries
        .iter()
        .find_map(|e| match e {
            AiEntryView::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(stored, manual(&text));
    // The model receives the message as stored.
    let requests = f.requests().await;
    let user = messages(&requests[0])
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap();
    assert_eq!(user["content"], stored);
}

#[test]
fn what_commands_and_mentions_attach_is_titled_and_shown_to_the_judge() {
    use crate::ai::judge::typed_text;
    let blocks = "<skill name=\"nginx\">\nRun nginx -t.\n</skill>\n\n\
                  <host_file host=\"web\" path=\"/etc/nginx/nginx.conf\" lines=\"2\">\na\nb\n\
                  </host_file>\n\n\
                  <host_directory host=\"web\" path=\"/etc/nginx\" entries=\"3\" truncated=\"true\">\
                  \nconf.d/\nnginx.conf\nsites/\n</host_directory>\n\n\
                  <mcp_resource server=\"docs\" uri=\"repo://x/a.md\" lines=\"1\">\n# A\n\
                  </mcp_resource>\n\n\
                  <mcp_prompt server=\"github\" name=\"review_pr\">\nReview PR 42\n</mcp_prompt>\n\n";
    assert_eq!(
        title_of(&format!("{blocks}/nginx check @/etc/nginx/nginx.conf")),
        "/nginx check @/etc/nginx/nginx.conf"
    );
    // Nothing typed: the first block's name, as the panel's mock titles it.
    assert_eq!(title_of(blocks), "/nginx");
    assert_eq!(
        title_of(
            "<host_file host=\"web\" path=\"/etc/nginx/nginx.conf\" lines=\"1\">\nx\n</host_file>"
        ),
        "nginx.conf"
    );
    assert_eq!(
        title_of("<host_directory host=\"web\" path=\"/etc\" entries=\"0\">\n\n</host_directory>"),
        "/etc"
    );
    assert_eq!(
        title_of("<mcp_prompt server=\"g\" name=\"triage\">\nx\n</mcp_prompt>"),
        "triage"
    );
    // The judge reads each as a line that names it, never what it holds (AI-47).
    assert_eq!(
        typed_text(&format!("{blocks}check it")),
        "[skill /nginx run]\n[file /etc/nginx/nginx.conf on the host, 2 lines]\n\
         [listing of the directory /etc/nginx on the host, 3 entries]\n\
         [MCP resource repo://x/a.md of docs, 1 line]\n[MCP prompt review_pr of github run]\n\n\
         check it"
    );
    // Other attributes, or another order, are typed text.
    for text in [
        "<skill>\nx\n</skill>\n\nq",
        "<host_file path=\"/x\" host=\"h\" lines=\"1\">\nx\n</host_file>\n\nq",
        "<host_directory host=\"h\" path=\"/\" entries=\"x\">\nx\n</host_directory>\n\nq",
        "<mcp_prompt name=\"p\">\nx\n</mcp_prompt>\n\nq",
    ] {
        assert_eq!(typed_text(text), text, "{text}");
    }
}
