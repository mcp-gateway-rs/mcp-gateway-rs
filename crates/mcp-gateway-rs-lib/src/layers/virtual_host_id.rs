use axum::{body::Body, middleware::Next, response::Response};
use http::{StatusCode, header};

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct VirtualHostId {
    value: String,
}

impl VirtualHostId {
    pub fn value(&self) -> &String {
        &self.value
    }
}

pub async fn virtual_host_id_layer(
    mut request: http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let uri = request.uri();

    if let Some(virtual_host_id) = extract_virtual_host_id(uri.path()) {
        request.extensions_mut().insert(virtual_host_id);
        next.run(request).await
    } else {
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("Problem occured retrieving the configuration"))
            .unwrap()
    }
}

fn extract_virtual_host_id(path: &str) -> Option<VirtualHostId> {
    if !path.starts_with("/mcp") {
        let parts: Vec<&str> = path.split("/mcp").collect();
        if let Some(f) = parts.first()
            && (f.starts_with('/') && f[1..].split('/').collect::<Vec<_>>().len() == 1)
        {
            parts.first().map(|vh| VirtualHostId {
                value: vh[1..].to_owned(),
            })
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::layers::virtual_host_id::{VirtualHostId, extract_virtual_host_id};

    #[test]
    fn test_virutal_host_extractor() {
        assert_eq!(None, extract_virtual_host_id("/mcp"));
        assert_eq!(
            Some(VirtualHostId {
                value: "12345_abcd-efgh".to_owned()
            }),
            extract_virtual_host_id("/12345_abcd-efgh/mcp")
        );
        assert_eq!(
            None,
            extract_virtual_host_id("/12345_abcd-efgh/12345_abcd-efgh/mcp")
        );
        assert_eq!(
            None,
            extract_virtual_host_id("//12345_abcd-efgh/12345_abcd-efgh/mcp")
        );
    }
}
