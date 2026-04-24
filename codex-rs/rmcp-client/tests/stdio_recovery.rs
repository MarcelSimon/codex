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

fn expected_echo_result(message: &str) -> CallToolResult {
    CallToolResult {
        content: Vec::new(),
        structured_content: Some(json!({
            "echo": format!("ECHOING: {message}"),
            "env": null,
        })),
        is_error: Some(false),
        meta: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn stdio_transport_closed_recovers_and_retries_once() -> anyhow::Result<()> {
    let client = create_client(Some(HashMap::from([(
        OsString::from("MCP_TEST_EXIT_AFTER_ECHO_CALLS"),
        OsString::from("1"),
    )])))
    .await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    sleep(Duration::from_millis(100)).await;

    let recovered = call_echo_tool(&client, "recovered").await?;
    assert_eq!(recovered, expected_echo_result("recovered"));

    Ok(())
}
