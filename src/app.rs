use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::routing::{delete, get, post};
use axum::Router;
use tower::ServiceBuilder;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::auth::middleware::{require_auth, require_auth_onboarding_exempt};
use crate::config::Config;
use crate::routes;
use crate::state::AppState;

pub fn build_app(config: &Config, state: AppState) -> Router {
    let request_timeout = config.request_timeout;
    let timeout_layer = middleware::from_fn(
        move |request: Request<Body>, next: Next| async move {
            match tokio::time::timeout(request_timeout, next.run(request)).await {
                Ok(response) => Ok(response),
                Err(_) => Err(StatusCode::REQUEST_TIMEOUT),
            }
        },
    );
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(
            config
                .cors_origins
                .iter()
                .map(|origin| origin.parse().expect("invalid CORS_ORIGIN entry"))
                .collect::<Vec<_>>(),
        );

    let common_layers = ServiceBuilder::new()
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(cors)
        .layer(timeout_layer)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(256 * 1024));

    let onboarding = Router::new()
        .route(
            "/auth/complete-onboarding",
            post(routes::auth::complete_onboarding),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_auth_onboarding_exempt,
        ));

    let protected = Router::new()
        .route("/settings", get(routes::settings::get_settings).patch(routes::settings::patch_settings))
        .route("/money-context", get(routes::money_context::get_money_context))
        .route(
            "/income-schedules",
            get(routes::income_schedules::list_schedules).post(routes::income_schedules::create_schedule),
        )
        .route(
            "/income-schedules/{id}",
            get(routes::income_schedules::get_schedule)
                .patch(routes::income_schedules::update_schedule)
                .delete(routes::income_schedules::delete_schedule),
        )
        .route(
            "/income",
            get(routes::income::list_income).post(routes::income::create_income),
        )
        .route(
            "/income/{id}",
            get(routes::income::get_income)
                .patch(routes::income::update_income)
                .delete(routes::income::delete_income),
        )
        .route(
            "/expenses",
            get(routes::expenses::list_expenses).post(routes::expenses::create_expense),
        )
        .route(
            "/expenses/period-view",
            get(routes::expenses::get_expense_period_view),
        )
        .route(
            "/expenses/upcoming-payable",
            get(routes::expenses::get_upcoming_payable),
        )
        .route("/expenses/early-pay", post(routes::expenses::early_pay_expense))
        .route(
            "/expenses/{id}",
            get(routes::expenses::get_expense)
                .patch(routes::expenses::patch_expense)
                .delete(routes::expenses::delete_expense),
        )
        .route(
            "/recurring-expenses",
            get(routes::recurring_expenses::list_recurring)
                .post(routes::recurring_expenses::create_recurring),
        )
        .route(
            "/recurring-expenses/{id}",
            get(routes::recurring_expenses::get_recurring)
                .patch(routes::recurring_expenses::update_recurring)
                .delete(routes::recurring_expenses::delete_recurring),
        )
        .route(
            "/recurring-expenses/{id}/cancel-reminder",
            post(routes::recurring_expenses::enable_cancel_reminder)
                .delete(routes::recurring_expenses::disable_cancel_reminder),
        )
        .route(
            "/subscription-reminders",
            get(routes::subscription_reminders::list_reminders),
        )
        .route(
            "/subscription-reminders/{id}/dismiss",
            post(routes::subscription_reminders::dismiss_reminder),
        )
        .route(
            "/planned-expenses",
            get(routes::planned_expenses::list_planned).post(routes::planned_expenses::create_planned),
        )
        .route(
            "/planned-expenses/{id}",
            get(routes::planned_expenses::get_planned)
                .patch(routes::planned_expenses::update_planned)
                .delete(routes::planned_expenses::delete_planned),
        )
        .route(
            "/budgets",
            get(routes::budgets::list_budgets).post(routes::budgets::create_budget),
        )
        .route(
            "/budgets/{id}",
            get(routes::budgets::get_budget)
                .patch(routes::budgets::update_budget)
                .delete(routes::budgets::delete_budget),
        )
        .route(
            "/budgets/{id}/expenses",
            get(routes::budgets::list_budget_expenses).post(routes::budgets::create_budget_expense),
        )
        .route(
            "/budgets/{id}/expenses/{expense_id}",
            delete(routes::budgets::delete_budget_expense),
        )
        .route("/savings", get(routes::savings::list_savings))
        .route("/tags", get(routes::tags::list_tags))
        .route("/projections", get(routes::projections::get_projections))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let protected = if config.rate_limit_enabled {
        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(100)
                .burst_size(200)
                .finish()
                .expect("valid governor config"),
        );
        protected.layer(GovernorLayer::new(governor_conf))
    } else {
        protected
    };

    Router::new()
        .route("/health", get(routes::health::health))
        .nest("/api/v1", onboarding.merge(protected))
        .layer(common_layers)
        .with_state(state)
}
