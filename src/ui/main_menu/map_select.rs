use crate::core::{JumpyDefaultMatchRunner, MatchPlugin};
use crate::prelude::*;

use crate::ui::map_select::{map_select_menu, MapSelectAction};

#[cfg(not(target_arch = "wasm32"))]
use crate::ui::network_game::NetworkGameState;

use super::player_select::PlayerSelectState;
use super::MenuPage;

#[cfg(not(target_arch = "wasm32"))]
use bones_framework::networking::{GgrsSessionRunner, GgrsSessionRunnerInfo, NetworkMatchSocket};

/// Network message that may be sent when selecting a map.
#[derive(Serialize, Deserialize)]
pub enum MapSelectMessage {
    SelectMap(MapPoolNetwork),
}

pub fn widget(
    ui: In<&mut egui::Ui>,
    world: &World,
    meta: Root<GameMeta>,
    mut sessions: ResMut<Sessions>,
    mut session_options: ResMut<SessionOptions>,
    assets: Res<AssetServer>,

    #[cfg(not(target_arch = "wasm32"))] network_socket: Option<Res<NetworkMatchSocket>>,
) {
    let mut select_action = MapSelectAction::None;

    // Get map select action from network
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(MapSelectAction::SelectMap(map_meta)) =
        handle_match_setup_messages(&network_socket, &assets)
    {
        select_action = MapSelectAction::SelectMap(map_meta);
    }

    // If the `TEST_MAP` debug env var is present start the game with the map
    // matching the provided name.
    #[cfg(debug_assertions)]
    'test: {
        use std::env::{var, VarError};
        use std::sync::atomic::{AtomicBool, Ordering};
        static DEBUG_DID_CHECK_ENV_VARS: AtomicBool = AtomicBool::new(false);
        if DEBUG_DID_CHECK_ENV_VARS
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let test_map = match var("TEST_MAP") {
                Ok(name) => name,
                Err(VarError::NotPresent) => break 'test,
                Err(VarError::NotUnicode(err)) => {
                    warn!("Invalid TEST_MAP, not unicode: {err:?}");
                    break 'test;
                }
            };

            let asset_server = world.resource::<AssetServer>();
            let game_meta = asset_server.root::<GameMeta>();

            let get_map_handles = || {
                let mut map_handles = Vec::new();
                map_handles.extend(game_meta.core.stable_maps.iter().copied());
                for pack in asset_server.packs() {
                    let pack_meta = asset_server.get(pack.root.typed::<crate::PackMeta>());
                    map_handles.extend(pack_meta.maps.iter().copied());
                }
                map_handles
            };

            let Some(test_map) = get_map_handles()
                .into_iter()
                .find(|h| asset_server.get(*h).name == test_map)
            else {
                warn!("TEST_MAP not found: {test_map}");
                let available_names = super::handle_names_to_string(get_map_handles(), |h| {
                    asset_server.get(h).name.as_str()
                });
                warn!("Available map names: {available_names}");
                break 'test;
            };

            select_action = MapSelectAction::SelectMap(MapPool::from_single_map(test_map));
        }
    }

    // If no network action - update action from UI
    if matches!(select_action, MapSelectAction::None) {
        select_action = world.run_system(map_select_menu, ());

        #[cfg(not(target_arch = "wasm32"))]
        // Replicate local action
        replicate_map_select_action(&select_action, &network_socket, &assets);
    }

    match select_action {
        MapSelectAction::None => (),
        MapSelectAction::SelectMap(maps) => {
            session_options.delete = true;
            ui.ctx().set_state(MenuPage::Home);

            #[cfg(not(target_arch = "wasm32"))]
            let session_runner: Box<dyn SessionRunner> = match network_socket {
                Some(socket) => {
                    let random_seed = ui.ctx().get_state::<NetworkGameState>().random_seed();

                    Box::new(GgrsSessionRunner::<NetworkInputConfig>::new(
                        Some(FPS),
                        GgrsSessionRunnerInfo::new(
                            socket.ggrs_socket(),
                            Some(meta.network.max_prediction_window),
                            Some(meta.network.local_input_delay),
                            random_seed,
                        ),
                    ))
                }
                None => Box::<JumpyDefaultMatchRunner>::default(),
            };
            #[cfg(target_arch = "wasm32")]
            let session_runner = Box::<JumpyDefaultMatchRunner>::default();

            let player_select_state = ui.ctx().get_state::<PlayerSelectState>();
            sessions.start_game(MatchPlugin {
                maps,
                player_info: std::array::from_fn(|i| {
                    let slot = player_select_state.slots[i];

                    PlayerInput {
                        active: !slot.is_empty(),
                        selected_player: slot
                            .selected_player()
                            .unwrap_or(player_select_state.players[0]),
                        selected_hat: slot.selected_hat(),
                        control_source: slot.user_control_source(),
                        editor_input: default(),
                        control: default(),
                        is_ai: slot.is_ai(),
                    }
                }),
                plugins: meta.get_plugins(&assets),
                session_runner,
                score: default(),
            });
            ui.ctx().set_state(PlayerSelectState::default());
        }
        MapSelectAction::GoBack => ui.ctx().set_state(MenuPage::PlayerSelect),
    }
}

/// Send a MapSelectMessage over network if local player has selected a map.
#[cfg(not(target_arch = "wasm32"))]
fn replicate_map_select_action(
    action: &MapSelectAction,
    socket: &Option<Res<NetworkMatchSocket>>,
    asset_server: &AssetServer,
) {
    use bones_framework::networking::SocketTarget;
    if let Some(socket) = socket {
        if let MapSelectAction::SelectMap(maps) = action {
            info!("Sending network SelectMap message.");
            socket.send_reliable(
                SocketTarget::All,
                &postcard::to_allocvec(&MapSelectMessage::SelectMap(
                    maps.into_network(asset_server),
                ))
                .unwrap(),
            );
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn handle_match_setup_messages(
    socket: &Option<Res<NetworkMatchSocket>>,
    asset_server: &AssetServer,
) -> Option<MapSelectAction> {
    if let Some(socket) = socket {
        let datas: Vec<(u32, Vec<u8>)> = socket.recv_reliable();

        for (_player, data) in datas {
            match postcard::from_bytes::<MapSelectMessage>(&data) {
                Ok(message) => match message {
                    MapSelectMessage::SelectMap(maps) => {
                        info!("Map select message received, starting game");

                        return Some(MapSelectAction::SelectMap(MapPool::from_network(
                            maps,
                            asset_server,
                        )));
                    }
                },
                Err(e) => {
                    // TODO: The second player in an online match is having this triggered by
                    // picking up a `ConfirmSelection` message, that might have been sent to
                    // _itself_.
                    warn!("Ignoring network message that was not understood: {e} data: {data:?}");
                }
            }
        }
    }

    None
}
