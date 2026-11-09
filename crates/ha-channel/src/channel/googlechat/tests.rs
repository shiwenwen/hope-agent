use super::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SPACE: &str = "spaces/synthetic";
const MESSAGE: &str = "spaces/synthetic/messages/reply";
const TEXT: &str = "**bold** [link](https://example.invalid) ~~old~~ <users/123>\n`<users/456>`";

async fn add_account(plugin: &GoogleChatPlugin, server: &MockServer, id: &str, standard: bool) {
    plugin.accounts.lock().await.insert(
        id.to_string(),
        RunningAccount {
            api: Arc::new(GoogleChatApi::for_test(server.uri())),
            cancel: CancellationToken::new(),
            standard_markdown: standard,
        },
    );
}

async fn create_reply(plugin: &GoogleChatPlugin, server: &MockServer, account: &str) -> String {
    Mock::given(method("POST"))
        .and(path(format!("/{SPACE}/messages")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": MESSAGE})),
        )
        .mount(server)
        .await;
    plugin
        .send_message(
            account,
            SPACE,
            &ReplyPayload {
                text: Some(TEXT.to_string()),
                thread_id: Some("spaces/synthetic/threads/original".to_string()),
                buttons: vec![vec![InlineButton {
                    text: "Approve".to_string(),
                    callback_data: Some("synthetic-approval".to_string()),
                    url: None,
                }]],
                ..ReplyPayload::text("")
            },
        )
        .await
        .unwrap()
        .message_id
        .unwrap()
}

#[tokio::test]
async fn explicit_syntax_matches_create_and_edit_under_both_cloud_defaults() {
    for standard in [false, true] {
        for cloud_default in ["MARKUP_SYNTAX_CHAT", "MARKUP_SYNTAX_MARKDOWN"] {
            let server = MockServer::start().await;
            let plugin = GoogleChatPlugin::new();
            add_account(&plugin, &server, "a", standard).await;
            let id = create_reply(&plugin, &server, "a").await;
            Mock::given(method("PATCH"))
                .and(path(format!("/{MESSAGE}")))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": MESSAGE})),
                )
                .mount(&server)
                .await;
            plugin
                .edit_message("a", SPACE, &id, &ReplyPayload::text(TEXT))
                .await
                .unwrap();
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 2);
            let create: serde_json::Value = requests[0].body_json().unwrap();
            let edit: serde_json::Value = requests[1].body_json().unwrap();
            let syntax = if standard {
                "MARKUP_SYNTAX_MARKDOWN"
            } else {
                "MARKUP_SYNTAX_CHAT"
            };
            assert_eq!(
                create["markupSyntax"], syntax,
                "cloud default: {cloud_default}"
            );
            assert_eq!(edit["markupSyntax"], syntax);
            let expected = if standard {
                format::compile_standard_markdown_mentions(TEXT)
            } else {
                format::markdown_to_googlechat(TEXT)
            };
            assert_eq!(create["text"], expected);
            assert_eq!(edit["text"], expected);
            assert_eq!(
                create["thread"]["name"],
                "spaces/synthetic/threads/original"
            );
            assert_eq!(create["cardsV2"][0]["cardId"], "approval");
            assert_eq!(
                requests[0].url.query(),
                Some("messageReplyOption=REPLY_MESSAGE_OR_FAIL")
            );
            assert_eq!(requests[1].url.query(), Some("updateMask=text"));
            assert!(edit.get("cardsV2").is_none());
            assert!(edit.get("thread").is_none());
        }
    }
}

#[tokio::test]
async fn in_flight_message_keeps_mode_when_account_switch_changes() {
    for original in [false, true] {
        let server = MockServer::start().await;
        let plugin = GoogleChatPlugin::new();
        add_account(&plugin, &server, "a", original).await;
        let id = create_reply(&plugin, &server, "a").await;
        add_account(&plugin, &server, "a", !original).await;
        Mock::given(method("PATCH"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": MESSAGE})),
            )
            .mount(&server)
            .await;
        plugin
            .edit_message("a", SPACE, &id, &ReplyPayload::text(TEXT))
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let first: serde_json::Value = requests[0].body_json().unwrap();
        let edit: serde_json::Value = requests[1].body_json().unwrap();
        assert_eq!(edit["markupSyntax"], first["markupSyntax"]);
        assert_eq!(edit["text"], first["text"]);
    }
}

#[tokio::test]
async fn unknown_message_or_wrong_account_or_space_does_not_mutate() {
    let server = MockServer::start().await;
    let plugin = GoogleChatPlugin::new();
    add_account(&plugin, &server, "a", true).await;
    add_account(&plugin, &server, "b", false).await;
    let id = create_reply(&plugin, &server, "a").await;
    for (account, space, message) in [
        ("b", SPACE, id.as_str()),
        ("a", "spaces/other", id.as_str()),
        ("a", SPACE, "spaces/synthetic/messages/old"),
    ] {
        assert!(plugin
            .edit_message(account, space, message, &ReplyPayload::text(TEXT))
            .await
            .is_err());
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn rejected_or_timed_out_edit_never_posts_a_replacement() {
    for status in [400, 429, 200] {
        let server = MockServer::start().await;
        let plugin = GoogleChatPlugin::new();
        add_account(&plugin, &server, "a", true).await;
        let id = create_reply(&plugin, &server, "a").await;
        let mut response =
            ResponseTemplate::new(status).set_body_json(serde_json::json!({"name": MESSAGE}));
        if status == 200 {
            response = response.set_delay(std::time::Duration::from_secs(3));
        }
        Mock::given(method("PATCH"))
            .respond_with(response)
            .mount(&server)
            .await;
        assert!(plugin
            .edit_message("a", SPACE, &id, &ReplyPayload::text(TEXT))
            .await
            .is_err());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
    }
}

#[tokio::test]
async fn same_resource_name_in_two_accounts_keeps_distinct_modes() {
    let server = MockServer::start().await;
    let plugin = GoogleChatPlugin::new();
    add_account(&plugin, &server, "a", true).await;
    add_account(&plugin, &server, "b", false).await;
    create_reply(&plugin, &server, "a").await;
    create_reply(&plugin, &server, "b").await;
    Mock::given(method("PATCH"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": MESSAGE})),
        )
        .mount(&server)
        .await;
    for account in ["a", "b"] {
        plugin
            .edit_message(account, SPACE, MESSAGE, &ReplyPayload::text(TEXT))
            .await
            .unwrap();
    }
    let requests = server.received_requests().await.unwrap();
    let a: serde_json::Value = requests[2].body_json().unwrap();
    let b: serde_json::Value = requests[3].body_json().unwrap();
    assert_eq!(a["markupSyntax"], "MARKUP_SYNTAX_MARKDOWN");
    assert_eq!(b["markupSyntax"], "MARKUP_SYNTAX_CHAT");
}

#[tokio::test]
async fn deleted_or_evicted_mode_cannot_fall_back_to_a_different_syntax() {
    let server = MockServer::start().await;
    let plugin = GoogleChatPlugin::new();
    add_account(&plugin, &server, "a", true).await;
    create_reply(&plugin, &server, "a").await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    plugin.delete_message("a", SPACE, MESSAGE).await.unwrap();
    assert!(plugin
        .edit_message("a", SPACE, MESSAGE, &ReplyPayload::text(TEXT))
        .await
        .is_err());
    create_reply(&plugin, &server, "a").await;
    {
        let mut cache = plugin.message_syntax.lock().await;
        let capacity = cache.cap().get();
        for i in 0..capacity {
            cache.put(
                ("other".to_string(), SPACE.to_string(), i.to_string()),
                MessageSyntax::Chat,
            );
        }
    }
    assert!(plugin
        .edit_message("a", SPACE, MESSAGE, &ReplyPayload::text(TEXT))
        .await
        .is_err());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(!requests.iter().any(|r| r.method == "PATCH"));
}
