//! OpenAPI documentation and router construction.

use axum::{
    routing::{delete, get, post},
    Router,
};
use utoipa::OpenApi;

use crate::adapters::inbound::loom::{
    discover_looms, get_knot_status, get_loom, get_loom_activity,
    get_loom_knots, list_looms, register_loom, unregister_loom,
};
use crate::adapters::inbound::system::{get_rig_config, health, list_agents};
use crate::adapters::inbound::types::AppContext;
use crate::application::ports::{KnotEventType, KnotState, ProcessingStatus};
use crate::application::usecases::{KnotStatus as KnotStatusDto, LoomSummary};
use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, Strand, StrandPath, TieOff, TieOffPath,
    TieOffStatus,
};
use crate::domain::events::{
    KnotRegistered, LoomEvent, ProcessingFailed, StrandEvent, TieOffProduced,
};
use crate::domain::value_objects::{AgentConfig, PromptTemplate, RigAgentConfig};

// ── OpenAPI / Swagger ──────────────────────────────────────────────────

/// OpenAPI document for the Knot API.
#[derive(utoipa::OpenApi, Clone)]
#[openapi(
    info(
        title = "Knot API",
        description = "Knot — local AI agent orchestration service",
        version = "0.1.0",
    ),
    paths(
        crate::adapters::inbound::system::health,
        crate::adapters::inbound::system::list_agents,
        crate::adapters::inbound::system::get_rig_config,
        crate::adapters::inbound::loom::list_looms,
        crate::adapters::inbound::loom::register_loom,
        crate::adapters::inbound::loom::unregister_loom,
        crate::adapters::inbound::loom::discover_looms,
        crate::adapters::inbound::loom::get_loom,
        crate::adapters::inbound::loom::get_loom_activity,
        crate::adapters::inbound::loom::get_loom_knots,
        crate::adapters::inbound::loom::get_knot_status,
    ),
    components(schemas(
        // Domain value objects
        RigAgentConfig,
        AgentConfig,
        PromptTemplate,
        // Domain entities
        LoomId,
        KnotId,
        StrandPath,
        TieOffPath,
        TieOffStatus,
        Knot,
        Loom,
        Strand,
        TieOff,
        // Domain events
        StrandEvent,
        LoomEvent,
        TieOffProduced,
        ProcessingFailed,
        KnotRegistered,
        // Application types
        LoomSummary,
        KnotStatusDto,
        ProcessingStatus,
        KnotEventType,
        KnotState,
        // Inbound types
        crate::adapters::inbound::types::RegisterLoomRequest,
        crate::adapters::inbound::types::KnotRequest,
        crate::adapters::inbound::types::RigConfigResponse,
    )),
)]
struct ApiDoc;

// ── Router builder ─────────────────────────────────────────────────────

/// Build the application router with loom routes and existing endpoints.
///
/// Accepts `AppContext` as shared state for all loom handlers.
pub fn build_app(ctx: AppContext) -> Router {
    let api_doc = ApiDoc::openapi();
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/swagger-ui/openapi.json", api_doc);

    Router::new()
        .merge(swagger)
        // Existing endpoints
        .route("/health", get(health))
        .route("/agents/{dir}", get(list_agents))
        // Config endpoints
        .route("/config/rig", get(get_rig_config))
        // Loom endpoints
        .route("/looms", get(list_looms))
        .route("/looms", post(register_loom))
        .route("/looms/discover", post(discover_looms))
        .route("/looms/{id}", get(get_loom))
        .route("/looms/{id}", delete(unregister_loom))
        .route("/looms/{id}/activity", get(get_loom_activity))
        .route("/looms/{id}/knots", get(get_loom_knots))
        .route("/looms/{id}/knots/{knot_name}", get(get_knot_status))
        .with_state(ctx)
}
