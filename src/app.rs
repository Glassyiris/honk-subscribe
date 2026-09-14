use std::{borrow::Cow, sync::Arc};

use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    Router,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    http::header,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    export::{ExportFormat, ExportLimits},
    graphql::{AppSchema, schema},
    service::Service,
    store::Store,
};

#[derive(rust_embed::RustEmbed)]
#[folder = "static/"]
struct StaticAssets;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<Service>,
    pub schema: AppSchema,
}

impl AppState {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        let store = Arc::new(Store::open(db_path)?);
        let service = Arc::new(Service::new(store)?);
        let schema = schema(service.clone());
        Ok(Self { service, schema })
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/graphql", get(graphql).post(graphql))
        .route("/graphiql", get(graphiql))
        .route("/subscription", get(subscription))
        .route("/subscription/{format}", get(subscription_format))
        .route("/health", get(health))
        .with_state(state)
        .route("/{*path}", get(static_asset))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

fn embedded_asset(path: &str) -> Option<(Cow<'static, [u8]>, &'static str)> {
    let path = path.trim_start_matches('/');
    let file = StaticAssets::get(path)?;
    Some((file.data, asset_content_type(path)))
}

async fn static_asset(Path(path): Path<String>) -> Response {
    let Some((body, content_type)) = embedded_asset(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ([(header::CONTENT_TYPE, content_type)], body.into_owned()).into_response()
}

fn asset_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

async fn graphiql() -> Html<String> {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

#[derive(Debug, Default)]
struct ExportQuery {
    format: Option<String>,
    limit: Option<usize>,
    region_limit: Option<usize>,
    provider_limit: Option<usize>,
    provider: Vec<String>,
    exclude_dead: bool,
}

impl ExportQuery {
    fn parse(raw: Option<&str>) -> Result<Self, String> {
        let mut query = Self::default();
        for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
            let value = value.into_owned();
            match key.as_ref() {
                "format" => query.format = Some(value),
                "limit" => query.limit = Some(parse_limit("limit", &value)?),
                "region_limit" | "regionLimit" => {
                    query.region_limit = Some(parse_limit("region_limit", &value)?);
                }
                "provider_limit" | "providerLimit" => {
                    query.provider_limit = Some(parse_limit("provider_limit", &value)?);
                }
                "provider" | "provider[]" | "providers" => query.provider.push(value),
                "exclude_dead" | "excludeDead" => query.exclude_dead = parse_bool(&value)?,
                _ => {}
            }
        }
        Ok(query)
    }

    fn limits(&self) -> ExportLimits {
        ExportLimits {
            total: self.limit,
            per_region: self.region_limit,
            per_provider: self.provider_limit,
            providers: self.provider.clone(),
            exclude_dead: self.exclude_dead,
        }
    }
}

async fn subscription(State(state): State<AppState>, RawQuery(raw_query): RawQuery) -> Response {
    let query = match ExportQuery::parse(raw_query.as_deref()) {
        Ok(query) => query,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    export_response(&state, query.format.as_deref(), query.limits())
}

async fn subscription_format(
    State(state): State<AppState>,
    Path(format): Path<String>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let query = match ExportQuery::parse(raw_query.as_deref()) {
        Ok(query) => query,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    export_response(&state, Some(&format), query.limits())
}

fn parse_limit(name: &str, value: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("{name} must be a positive integer"))
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err("exclude_dead must be a boolean".into()),
    }
}

fn export_response(state: &AppState, raw_format: Option<&str>, limits: ExportLimits) -> Response {
    let format = match ExportFormat::parse(raw_format) {
        Ok(format) => format,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    match state.service.export(format, limits) {
        Ok(body) => {
            let content_disposition =
                format!("attachment; filename=\"honk.{}\"", format.extension());
            (
                [
                    (header::CONTENT_TYPE, format.content_type()),
                    (header::CONTENT_DISPOSITION, content_disposition.as_str()),
                ],
                body,
            )
                .into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn graphql(State(state): State<AppState>, request: GraphQLRequest) -> GraphQLResponse {
    state.schema.execute(request.into_inner()).await.into()
}

async fn health(State(state): State<AppState>) -> Response {
    match state.service.health() {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ExportQuery, embedded_asset};

    #[test]
    fn embedded_frontend_contains_index() {
        let (body, content_type) = embedded_asset("index.html").expect("embedded index");
        assert!(body.starts_with(b"<!doctype html>"));
        assert_eq!(content_type, "text/html; charset=utf-8");
    }

    #[test]
    fn export_query_supports_dead_node_filter() {
        assert!(
            ExportQuery::parse(Some("exclude_dead=1"))
                .expect("valid dead-node filter")
                .limits()
                .exclude_dead
        );
        assert!(
            !ExportQuery::parse(Some("exclude_dead=0"))
                .expect("valid disabled dead-node filter")
                .limits()
                .exclude_dead
        );
        assert!(ExportQuery::parse(Some("exclude_dead=maybe")).is_err());
    }
}
