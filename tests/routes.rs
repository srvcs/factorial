use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_factorial::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

/// Mock `srvcs-multiply` that actually COMPUTES `a * b` from the request body.
///
/// This is required to genuinely test the counted-loop fold: a fixed response
/// would let a broken loop still "pass". Reads `{ "a", "b" }`, returns
/// `{ "a", "b", "result": a*b }`.
async fn spawn_multiply() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|Json(body): Json<Value>| async move {
            let a = body.get("a").and_then(Value::as_i64).unwrap_or(0);
            let b = body.get("b").and_then(Value::as_i64).unwrap_or(0);
            Json(json!({ "a": a, "b": b, "result": a * b }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Mock dependency answering `POST /` with a fixed status + body.
async fn spawn_mock(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(multiply_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            multiply_url: multiply_url.to_string(),
        },
    )
}

async fn eval(multiply_url: &str, value: Value) -> (StatusCode, Value) {
    let res = app(multiply_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": value }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const DEAD_URL: &str = "http://127.0.0.1:1";

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

#[tokio::test]
async fn factorial_of_five_is_120() {
    let multiply = spawn_multiply().await;
    let (status, body) = eval(&multiply, json!(5)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["value"], 5);
    assert_eq!(body["result"], 120);
}

#[tokio::test]
async fn factorial_of_zero_is_one() {
    // The loop body never runs, so the multiply dependency is never called;
    // point it at a dead URL to prove that.
    let (status, body) = eval(DEAD_URL, json!(0)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 1);
}

#[tokio::test]
async fn factorial_of_one_is_one() {
    // Likewise, 1! makes no multiply calls.
    let (status, body) = eval(DEAD_URL, json!(1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 1);
}

#[tokio::test]
async fn larger_factorial_folds_correctly() {
    let multiply = spawn_multiply().await;
    let (status, body) = eval(&multiply, json!(6)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 720);
}

#[tokio::test]
async fn negative_is_rejected() {
    let multiply = spawn_multiply().await;
    let (status, body) = eval(&multiply, json!(-3)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "factorial of a negative number");
}

#[tokio::test]
async fn non_integer_value_is_rejected() {
    let multiply = spawn_multiply().await;
    let (status, _) = eval(&multiply, json!(4.5)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn degrades_when_multiply_is_unreachable() {
    // 5! needs the loop, so an unreachable multiply must surface as 503.
    let (status, body) = eval(DEAD_URL, json!(5)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-multiply");
}

#[tokio::test]
async fn forwards_dependency_422() {
    let multiply = spawn_mock(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not a number" }),
    )
    .await;
    let (status, _) = eval(&multiply, json!(5)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
