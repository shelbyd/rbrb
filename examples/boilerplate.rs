use rbrb::{PlayerId, PlayerInputs, Request, SessionBuilder};

use macroquad::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr, ops::ControlFlow, time::Duration};
use structopt::StructOpt;

#[derive(StructOpt)]
struct Options {
    #[structopt(long, default_value = "7000")]
    local_port: u16,

    #[structopt(long)]
    local_index: PlayerId,

    remote_players: Vec<SocketAddr>,
}

#[derive(Default, Serialize, Deserialize)]
struct GameState {
    value: isize,
    player_cooldowns: BTreeMap<PlayerId, Duration>,
}

#[derive(Serialize, Deserialize, Debug)]
enum Action {
    Increment,
    Decrement,
}

#[macroquad::main("Basic RbRb")]
async fn main() {
    let options = Options::from_args();

    let mut session = SessionBuilder::default()
        .remote_players(&options.remote_players)
        .local_player(options.local_index, options.local_port)
        .step_size(Duration::from_millis(16))
        .start()
        .unwrap();

    let mut game_state = GameState::default();
    loop {
        while let ControlFlow::Continue(()) =
            session.next_request(|request: Request<'_>| match request {
                Request::SaveTo(vec) => bincode::serialize_into(vec, &game_state).unwrap(),
                Request::CaptureLocalInput(vec) => {
                    let input = if is_key_down(KeyCode::Up) {
                        Some(Action::Increment)
                    } else if is_key_down(KeyCode::Down) {
                        Some(Action::Decrement)
                    } else {
                        None
                    };
                    bincode::serialize_into(vec, &input).unwrap();
                }
                Request::Advance(dt, inputs) => {
                    game_state.player_cooldowns = game_state
                        .player_cooldowns
                        .iter()
                        .filter_map(|(k, v)| if *v < dt { None } else { Some((*k, *v - dt)) })
                        .collect();

                    let inputs: PlayerInputs<Option<Action>> =
                        inputs.map(|vec| bincode::deserialize_from(&vec[..]).unwrap());
                    for (player_id, action) in inputs.iter() {
                        let on_cooldown = game_state.player_cooldowns.contains_key(&player_id);
                        if on_cooldown {
                            continue;
                        }

                        if let Some(action) = action {
                            match action {
                                Action::Increment => game_state.value += 1,
                                Action::Decrement => game_state.value -= 1,
                            }
                            game_state
                                .player_cooldowns
                                .insert(*player_id, Duration::from_millis(300));
                        }
                    }
                }
                unhandled => {
                    panic!("unhandled request: {:?}", unhandled);
                }
            })
        {}

        draw_text(&format!("{}", game_state.value), 20.0, 20.0, 20.0, WHITE);

        next_frame().await;
    }
}
