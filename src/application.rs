use crate::{
    arena::{Arena, opponent, seed, snapshot},
    capacity::Registry,
    protocol::game::{privateapi as q, publicapi as p},
    rpc::{self, Context, Failure},
    transport::{Peer, Role},
};
use axum::{
    Extension, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
};
use std::sync::Arc;
use tokio::sync::Mutex;
#[derive(Clone)]
pub struct Application {
    pub arena: Arc<Arena>,
    pub private_client: reqwest::Client,
    pub private_url: String,
    pub version: String,
    pub registry: Arc<Mutex<Registry>>,
}
pub fn router(state: Application) -> Router {
    Router::new()
        .route("/v1/public/{method}", post(public))
        .route("/v1/control/{method}", post(control))
        .fallback(|| async { Failure::NOT_FOUND })
        .layer(DefaultBodyLimit::max(rpc::MAX_BODY))
        .with_state(state)
}
async fn control(
    State(state): State<Application>,
    Extension(peer): Extension<Peer>,
    Path(method): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    if peer.role != Role::Coordinator {
        return Err(Failure::FORBIDDEN);
    }
    match method.as_str() {
        "GetGameServerCapacity" => {
            let _: q::GetGameServerCapacityRequest = rpc::decode(&headers, body)?;
            Ok(rpc::response(state.registry.lock().await.capacity()))
        }
        "StartGuildBattlePreload" => {
            let _: q::StartGuildBattlePreloadRequest = rpc::decode(&headers, body)?;
            // GuildBattle admission remains closed until its durable replay/runtime pipeline is connected.
            Err(Failure::UNAVAILABLE)
        }
        _ => Err(Failure::NOT_FOUND),
    }
}
async fn public(
    State(state): State<Application>,
    Extension(peer): Extension<Peer>,
    Path(method): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    if peer.role != Role::PublicApi {
        return Err(Failure::FORBIDDEN);
    }
    let context = Context::from_headers(&headers)?;
    match method.as_str() {
        "UpdateArenaParty" => state.update(context, rpc::decode(&headers, body)?).await,
        "StartArenaBattle" => state.start(context, rpc::decode(&headers, body)?).await,
        _ => Err(Failure::NOT_FOUND),
    }
}
impl Application {
    async fn private<Q: prost::Message, R: prost::Message + Default>(
        &self,
        method: &str,
        request: Q,
        context: &Context,
    ) -> Result<R, Failure> {
        rpc::call(
            &self.private_client,
            &self.private_url,
            &format!("/v1/private/{method}"),
            request,
            Some(context),
            None,
        )
        .await
    }
    async fn update(
        &self,
        context: Context,
        r: p::UpdateArenaPartyRequest,
    ) -> Result<Response, Failure> {
        if r.player_id != context.player_id {
            return Err(Failure::FORBIDDEN);
        }
        self.arena
            .prepare(context.player_id, r.formation_id, &r.characters, 0)?;
        let _: q::SaveArenaPartyResponse = self
            .private(
                "SaveArenaParty",
                q::SaveArenaPartyRequest {
                    player_id: context.player_id,
                    formation_id: r.formation_id,
                    characters: r.characters,
                },
                &context,
            )
            .await?;
        Ok(rpc::response(p::UpdateArenaPartyResponse {}))
    }
    async fn start(&self, context: Context, r: p::ArenaBattleRequest) -> Result<Response, Failure> {
        if r.player_id != context.player_id {
            return Err(Failure::FORBIDDEN);
        }
        if r.client_version.is_empty() {
            return Err(Failure::BAD_REQUEST);
        }
        if r.client_version != self.version {
            let mut response = rpc::response(p::ClientVersionMismatchResponse {
                error_code: 24,
                required_version: self.version.clone(),
            });
            *response.status_mut() = StatusCode::CONFLICT;
            return Ok(response);
        }
        let mode = r
            .mode
            .filter(|v| *v == 0 || *v == 1)
            .ok_or(Failure::BAD_REQUEST)?;
        self.arena.prepare(
            context.player_id,
            r.local_formation_id,
            &r.local_characters,
            0,
        )?;
        let seed = seed()?;
        let enemy_id = if mode == 0 {
            let ids: q::GetAllPlayerIDsResponse = self
                .private("GetAllPlayerIDs", q::GetAllPlayerIDsRequest {}, &context)
                .await?;
            let Some(id) = opponent(ids.player_id, context.player_id, seed)? else {
                return Ok(arena_error(0));
            };
            id
        } else {
            if !rpc::ordinary_id(r.opponent_id) || r.opponent_id == context.player_id {
                return Err(Failure::BAD_REQUEST);
            }
            r.opponent_id
        };
        let own: q::GetArenaBattleDataResponse = self
            .private(
                "GetArenaBattleData",
                q::GetArenaBattleDataRequest {
                    player_id: context.player_id,
                },
                &context,
            )
            .await?;
        if !own.arena_party_registered {
            return Ok(arena_error(3));
        }
        let enemy: q::GetArenaBattleDataResponse = self
            .private(
                "GetArenaBattleData",
                q::GetArenaBattleDataRequest {
                    player_id: enemy_id,
                },
                &context,
            )
            .await?;
        if !enemy.player_exists {
            return Ok(arena_error(1));
        }
        if !enemy.arena_party_registered {
            return Ok(arena_error(2));
        }
        if mode == 1 {
            let friends: q::ListFriendsPrivateResponse = self
                .private(
                    "ListFriendsPrivate",
                    q::ListFriendsPrivateRequest {
                        player_id: context.player_id,
                    },
                    &context,
                )
                .await?;
            if !friends.friends.iter().any(|p| p.player_id == enemy_id) {
                return Ok(arena_error(4));
            }
        }
        let own_party =
            self.arena
                .prepare(context.player_id, own.formation_id, &own.characters, 0)?;
        let enemy_party = self
            .arena
            .prepare(enemy_id, enemy.formation_id, &enemy.characters, 1)?;
        let enemy_characters = enemy_party
            .iter()
            .zip(&enemy.characters)
            .map(|(c, e)| snapshot(c, e))
            .collect();
        let arena = self.arena.clone();
        tokio::task::spawn_blocking(move || arena.simulate(own_party, enemy_party, seed))
            .await
            .map_err(|_| Failure::UNAVAILABLE)??;
        Ok(rpc::response(p::ArenaBattleResponse {
            own_party_matched: r.local_formation_id == own.formation_id
                && r.local_characters == own.characters,
            own_formation_id: own.formation_id,
            own_characters: own.characters,
            enemy_formation_id: enemy.formation_id,
            enemy_characters,
            seed,
            enemy_player_id: enemy_id,
        }))
    }
}
fn arena_error(code: i32) -> Response {
    let mut response = rpc::response(p::ArenaBattleErrorResponse { error_code: code });
    *response.status_mut() = StatusCode::CONFLICT;
    response
}
