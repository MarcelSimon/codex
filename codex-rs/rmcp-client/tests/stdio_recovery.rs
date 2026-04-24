use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_rmcp_client::ElicitationAction;
use codex_rmcp_client::ElicitationResponse;
use codex_rmcp_client::LocalStdioServerLauncher;
use codex_rmcp_client::RmcpClient;
use codex_utils_cargo_bin::CargoBinError;
use futures::FutureExt as _;
use pretty_assertions::assert_eq;
use rmcp::model::CallToolResult;
use rmcp::model::ClientCapabilities;
use rmcp::model::ElicitationCapability;
use rmcp::model::FormElicitationCapability;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::ListToolsResult;
use rmcp::model::ProtocolVersion;
use serde_json::json;
use tokio::time::sleep;

fn stdio_server_bin() -> Result<PathBuf, CargoBinError> {
    codex_utils_cargo_bin::cargo_bin("test_stdio_server")
}

fn init_params() -> InitializeRequestParams {
    InitializeRequestParams {
        meta: None,
        capabilities: ClientCapabilities {
            experimental: None,
            extensions: None,
            roots: None,
            sampling: None,
            elicitation: Some(ElicitationCapability {
                form: Some(FormElicitationCapability {
                    schema_validation: None,
                }),
                url: None,
            }),
            tasks: None,
        },
        client_info: Implementation {
            name: "codex-test".into(),
            version: "0.0.0-test".into(),
            title: Some("Codex rmcp stdio recovery test".into()),
            description: None,
            icons: None,
            website_url: None,
        },
        protocol_version: ProtocolVersion::V_2025_06_18,
    }
}

async fn create_client(env: Option<HashMap<OsString, OsString>>) -> anyhow::Result<RmcpClient> {
    let client = RmcpClient::new_stdio_client(
        stdio_server_bin()?.into(),
        Vec::<OsString>::new(),
        env,
        &[],
        /*cwd*/ None,
        Arc::new(LocalStdioServerLauncher::new(std::env::current_dir()?)),
    )
    .await?;

    client
        .initialize(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await?;

    Ok(client)
}

async fn call_echo_tool(client: &RmcpClient, message: &str) -> anyhow::Result<CallToolResult> {
    client
        .call_tool(
            "echo".to_string(),
            Some(json!({ "message": message })),
            /*meta*/ None,
            Some(Duration::from_secs(5)),
        )
        .await
}

async fn list_tools(client: &RmcpClient) -> anyhow::Result<ListToolsResult> {
    client
        .list_tools(/*params*/ None, Some(Duration::from_secs(5)))
        .await
}

fn assert_has_echo_tool(result: &ListToolsResult) {
    assert!(
        result.tools.iter().any(|tool| tool.name == "echo"),
        "expected echo tool in {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stdio_transport_closed_replays_safe_list_tools_once() -> anyhow::Result<()> {
    let client = create_client(Some(HashMap::from([(
        OsString::from("MCP_TEST_EXIT_AFTER_LIST_TOOLS_CALLS"),
        OsString::from("1"),
    )])))
    .await?;

    let warmup = list_tools(&client).await?;
    assert_has_echo_tool(&warmup);

    sleep(Duration::from_millis(100)).await;

    let recovered = list_tools(&client).await?;
    assert_has_echo_tool(&recovered);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stdio_transport_closed_reconnects_without_replaying_tool_calls() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let count_file = temp_dir.path().join("echo-count");
    let client = create_client(Some(HashMap::from([
        (
            OsString::from("MCP_TEST_EXIT_DURING_ECHO"),
            OsString::from("1"),
        ),
        (
            OsString::from("MCP_TEST_ECHO_CALL_COUNT_FILE"),
            OsString::from(count_file.as_os_str()),
        ),
    ])))
    .await?;

    let err = call_echo_tool(&client, "side-effect").await.expect_err(
        "ambiguous transport close during a tool call should reconnect but return the original error",
    );
    assert!(
        !err.to_string().is_empty(),
        "expected a non-empty tool call error"
    );
    assert_eq!(std::fs::read_to_string(&count_file)?, "1");

    let recovered = list_tools(&client).await?;
    assert_has_echo_tool(&recovered);
    assert_eq!(std::fs::read_to_string(&count_file)?, "1");

    Ok(())
}
