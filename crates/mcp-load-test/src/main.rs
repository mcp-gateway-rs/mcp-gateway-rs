use std::time::Duration;

use goose::prelude::*;
use reqwest::RequestBuilder;

pub const INITIALIZE_MCP_SESSION: &'static str = r#"{"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{"sampling":{},"elicitation":{},"roots":{"listChanged":true},"tasks":{"list":{},"cancel":{},"requests":{"sampling":{"createMessage":{}},"elicitation":{"create":{}}}}},"clientInfo":{"name":"inspector-client","version":"0.21.1"}},"jsonrpc":"2.0","id":0}"#;
pub const NOTIFY_MCP_SESSION: &'static str = r#"{"method":"notifications/initialized","jsonrpc":"2.0"}"#;
//pub const COUNTER_ONE_INC: &'static str = r#"{"method":"tools/call","params":{"name":"increment","arguments":{},"_meta":{"progressToken":1}},"jsonrpc":"2.0","id":5}"#;
pub const COUNTER_ONE_INC: &'static str = r#"{"method":"tools/call","params":{"name":"counter-one-increment","arguments":{},"_meta":{"progressToken":1}},"jsonrpc":"2.0","id":1}"#;
pub const MCP_ENDPOINT: &'static str = "/mcp-rs/servers/c0ffee00f001f00lf00ldeadbeefdead/mcp";
//pub const MCP_ENDPOINT: &'static str = "/mcp";

#[derive(Clone, Debug)]
struct Session {
    jwt_token: String,
    mcp_session_id: Option<String>,
}

async fn get_token(user: &mut GooseUser) -> TransactionResult {
    let response = match user.get("/mcp-rs/admin/tokens/admin@example.com").await?.response {
        Ok(r) => match r.text().await {
            Ok(j) => j,
            Err(e) => return Err(Box::new(e.into())),
        },
        Err(e) => return Err(Box::new(e.into())),
    };

    user.set_session_data(Session { jwt_token: response, mcp_session_id: None });

    Ok(())
}

fn build_mcp_request(
    user: &mut GooseUser,
    session: &Session,
    body: &'static str,
    method: &GooseMethod,
) -> Result<RequestBuilder, Box<TransactionError>> {
    let reqwest_request_builder = user.get_request_builder(method, MCP_ENDPOINT)?.bearer_auth(&session.jwt_token);
    let request = reqwest_request_builder
        .body(body)
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json;application/json; text/event-stream");
    Ok(if let Some(mcp_session_id) = session.mcp_session_id.as_ref() {
        request.header("Mcp-session-id", mcp_session_id)
    } else {
        request
    })
}

fn initialize_mcp_request(user: &mut GooseUser, session: &Session) -> Result<RequestBuilder, Box<TransactionError>> {
    build_mcp_request(user, session, INITIALIZE_MCP_SESSION, &GooseMethod::Post)
}

fn notify_mcp_request(user: &mut GooseUser, session: &Session) -> Result<RequestBuilder, Box<TransactionError>> {
    build_mcp_request(user, session, NOTIFY_MCP_SESSION, &GooseMethod::Post)
}

fn counter_one_increase_request(
    user: &mut GooseUser,
    session: &Session,
) -> Result<RequestBuilder, Box<TransactionError>> {
    build_mcp_request(user, session, COUNTER_ONE_INC, &GooseMethod::Post)
}

fn end_session_mcp_request(user: &mut GooseUser, session: &Session) -> Result<RequestBuilder, Box<TransactionError>> {
    build_mcp_request(user, session, "", &GooseMethod::Delete)
}

async fn counter_call(user: &mut GooseUser) -> TransactionResult {
    let session = user.get_session_data_unchecked::<Session>().clone();

    if session.mcp_session_id.is_none() {
        _ = user.log_debug(&format!("counter_call: No Session id "), None, None, None)?;
        return Err(Box::new(TransactionError::Custom("no session id".to_owned())));
    }

    let calls = 1;
    for i in 0..calls {
        let counter_one_inc_request =
            GooseRequest::builder().set_request_builder(counter_one_increase_request(user, &session)?).build();
        tokio::time::sleep(Duration::from_millis(10)).await;
        match user.request(counter_one_inc_request).await?.response {
            Ok(r) => {
                if r.status() == http::StatusCode::OK {
                    if i == calls - 1 {
                        let payload = r.text().await?;

                        if payload.contains("\"isError\":false") && payload.contains(&format!("\"text\":\"{}\"", calls))
                        {
                            ()
                        } else {
                            let _ = user.log_debug(&format!("Couner has a problem "), None, None, Some(&payload));
                            return Err(Box::new(TransactionError::Custom("counting problem".to_owned())));
                        }
                    }
                } else {
                    return Err(Box::new(TransactionError::Custom("wrong response code ".to_owned())));
                }
            },
            Err(e) => return Err(Box::new(e.into())),
        };
    }

    Ok(())
}

async fn delete_session(user: &mut GooseUser) -> TransactionResult {
    let session = user.get_session_data_unchecked::<Session>().clone();
    if session.mcp_session_id.is_none() {
        return Err(Box::new(TransactionError::Custom("no session id".to_owned())));
    }
    let end_session = GooseRequest::builder().set_request_builder(end_session_mcp_request(user, &session)?).build();

    let mut session = user.get_session_data_unchecked::<Session>().clone();
    session.mcp_session_id = None;
    user.set_session_data(session);
    match user.request(end_session).await?.response {
        Ok(r) => {
            if r.status() == http::StatusCode::ACCEPTED || r.status() == http::StatusCode::OK {
                Ok(())
            } else {
                Err(Box::new(TransactionError::Custom("wrong response code ".to_owned())))
            }
        },
        Err(e) => Err(Box::new(e.into())),
    }
}

async fn initialize_session(user: &mut GooseUser) -> TransactionResult {
    let session = user.get_session_data_unchecked::<Session>();
    let mut session = session.clone();
    if session.mcp_session_id.is_some() {
        _ = user.log_debug(&format!("intialize has session id "), None, None, None);
        return Err(Box::new(TransactionError::Custom("initialize has session id  ".to_owned())));
    }
    let init_request = {
        let builder = initialize_mcp_request(user, &session)?;
        GooseRequest::builder().set_request_builder(builder).build()
    };

    let session_id = match user.request(init_request).await?.response {
        Ok(r) => {
            if r.status() == http::StatusCode::OK {
                if let Some(session_id) = r.headers().get("mcp-session-id").cloned() {
                    session_id.clone()
                } else {
                    return Err(Box::new(TransactionError::Custom("can't find mcp-session-id".to_owned())));
                }
            } else {
                return Err(Box::new(TransactionError::Custom("wrong response code ".to_owned())));
            }
        },
        Err(e) => return Err(Box::new(e.into())),
    };

    tokio::time::sleep(Duration::from_millis(10)).await;

    session.mcp_session_id = Some(session_id.to_str().unwrap_or_default().to_owned());
    let notify_request = GooseRequest::builder().set_request_builder(notify_mcp_request(user, &session)?).build();
    match user.request(notify_request).await?.response {
        Ok(r) => {
            if r.status() == http::StatusCode::ACCEPTED {
                ()
            } else {
                return Err(Box::new(TransactionError::Custom("wrong response code ".to_owned())));
            }
        },
        Err(e) => return Err(Box::new(e.into())),
    }
    user.set_session_data(session);
    tokio::time::sleep(Duration::from_millis(10)).await;

    Ok(())
}

async fn all_session(user: &mut GooseUser) -> TransactionResult {
    initialize_session(user).await?;
    counter_call(user).await?;
    delete_session(user).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("MCPCounter")
                .register_transaction(transaction!(get_token).set_on_start())
                .register_transaction(transaction!(initialize_session).set_name("MCP Init").set_sequence(1))
                .register_transaction(transaction!(counter_call).set_name("MCP Counter Call").set_sequence(2))
                .register_transaction(transaction!(delete_session).set_name("MCP Delete").set_sequence(3)), //.register_transaction(transaction!(all_session)),
        )
        .execute()
        .await?;

    Ok(())
}
