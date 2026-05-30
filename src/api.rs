use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-factorial";
pub const CONCERN: &str = "arithmetic: factorial";
pub const DEPENDS_ON: &[&str] = &["srvcs-multiply"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub multiply_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    #[schema(value_type = Object)]
    pub value: Value,
}

#[derive(Serialize, ToSchema)]
pub struct FactorialResponse {
    #[schema(value_type = Object)]
    pub value: Value,
    pub result: i64,
}

fn ok(value: Value, result: i64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "value": value, "result": result })),
    )
        .into_response()
}

fn invalid(reason: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": reason })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask `srvcs-multiply` for `a * b`, mapping its failures to the response this
/// service should return.
///
/// Returns `Ok(product)` on success, or an error `Response` (422/503) the caller
/// should return verbatim.
async fn ask_multiply(url: &str, a: i64, b: i64) -> Result<i64, Response> {
    let body = json!({ "a": a, "b": b });
    match client::call(url, &body).await {
        Err(DepError::Unreachable) => Err(degraded("srvcs-multiply")),
        Ok((200, body)) => body
            .get("result")
            .and_then(Value::as_i64)
            .ok_or_else(|| degraded("srvcs-multiply")),
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded("srvcs-multiply")),
    }
}

/// `POST /` — compute `value!` (factorial).
///
/// This service does no arithmetic of its own. It folds a counted loop over
/// `2..=value`, asking `srvcs-multiply` for each partial product. `0! == 1` and
/// `1! == 1` (the loop body never runs). A negative input has no factorial, so
/// it is rejected with `422`. If the multiply dependency is unreachable, this
/// service reports itself degraded (`503`) rather than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = FactorialResponse),
        (status = 422, description = "value is negative or not an integer"),
        (status = 500, description = "the multiply dependency returned a non-integer result"),
        (status = 503, description = "the multiply dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let value = match req.value.as_i64() {
        Some(n) => n,
        None => return invalid("value is not an integer"),
    };

    if value < 0 {
        return invalid("factorial of a negative number");
    }

    let mut acc: i64 = 1;
    for i in 2..=value {
        acc = match ask_multiply(&deps.multiply_url, acc, i).await {
            Ok(p) => p,
            Err(resp) => return resp,
        };
    }

    ok(req.value, acc)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, FactorialResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-factorial");
        assert_eq!(info.depends_on, vec!["srvcs-multiply"]);
    }

    #[test]
    fn negative_is_rejected_as_unprocessable() {
        let resp = invalid("factorial of a negative number");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
