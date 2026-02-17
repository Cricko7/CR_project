use super::*;

pub(crate) async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: state.service_name,
    })
}
