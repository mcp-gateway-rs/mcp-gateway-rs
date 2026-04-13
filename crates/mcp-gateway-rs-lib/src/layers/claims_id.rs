// use std::{
//     pin::Pin,
//     sync::Arc,
//     task::{Context, Poll},
// };

// use axum::{
//     body::Body,
//     extract::{FromRequest, FromRequestParts, State},
//     http::Request,
//     middleware::Next,
//     response::{IntoResponse, Response},
// };
// use axum_jwt_auth::{Claims, Decoder, JwtDecoder};
// use futures::{FutureExt, future::BoxFuture};
// use http::{StatusCode, header, request::Parts};
// use serde::{Deserialize, Serialize, de::DeserializeOwned};
// use tower::Service;

// use tower_layer::Layer;
// use tracing::{info, warn};

use axum::{
    body::Body,
    extract::{FromRequestParts, Request, State},
    middleware::Next,
    response::Response,
};
use axum_jwt_auth::Claims;
use http::{StatusCode, header};
use tracing::warn;

use crate::common::{McpGatewayAppState, McpGatewayClaims};

// #[derive(Clone)]
// pub struct ClaimsLayer {
//     pub decoder: Arc<dyn JwtDecoder<McpClaims> + Send + Sync + 'static>,
// }

// impl<S> Layer<S> for ClaimsLayer {
//     type Service = ClaimsService<S>;

//     fn layer(&self, service: S) -> Self::Service {
//         ClaimsService {
//             service,
//             decoder: Arc::clone(&self.decoder),
//         }
//     }
// }

// #[derive(Clone)]
// pub struct ClaimsService<S> {
//     service: S,
//     decoder: Arc<dyn JwtDecoder<McpClaims> + Send + Sync>,
// }

// impl<S, B> Service<Request<B>> for ClaimsService<S>
// where
//     S: Service<Request<B>> + Send + Sync + 'static + Clone,
//     B: Send + Sync + 'static,
// {
//     type Response = S::Response;
//     type Error = S::Error;
//     type Future =
//         Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + Sync + 'static>>;
//     //type Future = Box<dyn Future<Output = <S as Service<Request<B>>>::Future> + 'static>

//     //Result<<S as Service<Request<B>>>::Response, <S as Service<Request<B>>>::Error>,

//     fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
//         self.service.poll_ready(cx)
//     }

//     fn call(&mut self, request: Request<B>) -> Self::Future {
//         let (mut parts, body) = request.into_parts();
//         let mut new_parts = parts.clone();
//         let mut new_self = self.clone();
//         let future = async {
//             // if let Ok(claims) =
//             //     Claims::<McpClaims>::from_request_parts(&mut new_parts, &new_self.decoder).await
//             // {
//             //     parts.extensions.insert(claims.claims);
//             // };
//             let request = Request::from_parts(parts, body);
//             let response = new_self.service.call(request).await;
//             response
//         };

//         Box::pin(future)
//     }
// }

pub async fn claims_layer(State(state): State<McpGatewayAppState>, request: http::Request<axum::body::Body>, next: Next) -> Response {
    let (mut parts, body) = request.into_parts();
    let mut new_parts = parts.clone();
    let decoder = state.jwt_token_decoder;
    let maybe_claims = Claims::<McpGatewayClaims>::from_request_parts(&mut new_parts, &decoder).await;
    if let Ok(claims) = maybe_claims {
        parts.extensions.insert(claims.claims);
        let request = Request::from_parts(parts, body);
        next.run(request).await
    } else {
        let err = maybe_claims.err();
        warn!("No claims {:?}", err);
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(format!("Invalid authorization token {err:?}")))
            .expect("Expecting this to work")
    }
}
