use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::{Router, get, post},
};
use chrono::{Duration, Utc};

use http::{StatusCode, header};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

use serde::{Deserialize, Serialize};
use std::fs;
//use tracing::debug;

use url::Url;

use crate::{
    common::{MCP_AUDIENCE, McpGatewayAppState},
    user_config_store::UserConfig,
};

#[derive(Deserialize, Serialize)]
struct DefaultClaims {
    iss: Url,
    sub: String,
    aud: String,
    exp: i64,
    iat: Option<i64>,
    userinfo: openid::Userinfo,
}

impl DefaultClaims {
    fn new(user_id: String) -> Self {
        let url = "http://mcp-gateway-rs".parse().unwrap();
        let audience = MCP_AUDIENCE.to_owned();
        let user_info = openid::Userinfo {
            sub: user_id.clone(),
            ..Default::default()
        };
        Self {
            iss: url,
            sub: user_id,
            aud: audience,
            exp: (Utc::now() + Duration::hours(1)).timestamp(),
            iat: Some(Utc::now().timestamp()),

            userinfo: user_info,
        }
    }
}

pub fn add_tools(router: Router<McpGatewayAppState>) -> Router<McpGatewayAppState> {
    router
        .route("/token/{user_id}", get(get_token))
        .route("/userconfigs/{user_id}", post(configure_user))
}

pub async fn get_token(
    State(state): State<McpGatewayAppState>,
    Path(user_id): Path<String>,
) -> Response {
    let key =
        EncodingKey::from_rsa_pem(&fs::read(&state.config.token_verification_private_key).unwrap())
            .unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test".to_string());

    let claims = DefaultClaims::new(user_id);

    let token = encode::<DefaultClaims>(&header, &claims, &key).unwrap();

    token.into_response()
}

//#[debug_handler]
pub async fn configure_user(
    Path(user_id): Path<String>,
    State(state): State<McpGatewayAppState>,
    Json(user_config): Json<UserConfig>,
) -> Response {
    if state
        .config_store
        .set_config(&user_id, &user_config)
        .await
        .is_ok()
    {
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("Added"))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("Problem with encoding "))
            .unwrap()
    }
}
