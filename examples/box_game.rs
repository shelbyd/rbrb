use rbrb::{
    BadSocket, BandwidthRecordingSocket, BasicUdpSocket, PlayerId, PlayerInputs, Request,
    SessionBuilder,
};

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

    #[structopt(long)]
    bad_network: bool,

    remote_players: Vec<SocketAddr>,
}

#[derive(Default, Serialize, Deserialize)]
struct GameState {
    box_positions: BTreeMap<PlayerId, Vec2>,
}

#[derive(Default, Serialize, Deserialize)]
struct Vec2 {
    x: f32,
    y: f32,
}

fn window_conf() -> Conf {
    let options = Options::from_args();

    Conf {
        window_title: format!(
            "Basic RbRb{}",
            if options.bad_network {
                " [Bad Network]"
            } else {
                ""
            }
        ),
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    env_logger::init();
    let options = Options::from_args();

    let builder = SessionBuilder::default()
        .remote_players(&options.remote_players)
        .local_player(options.local_index, options.local_port)
        .step_size(Duration::from_millis(17))
        .default_inputs(bincode::serialize(&Vec2::default()).unwrap());

    let builder = if options.bad_network {
        let s = BandwidthRecordingSocket::new(BadSocket::bind(options.local_port).unwrap());
        builder.with_socket(s)
    } else {
        let s = BandwidthRecordingSocket::new(BasicUdpSocket::bind(options.local_port).unwrap());
        builder.with_socket(s)
    };

    let mut session = builder.start().unwrap();

    let mut game_state = GameState::default();
    for (id, _type) in session.players() {
        game_state.box_positions.entry(id).or_insert_with(|| Vec2 {
            x: id as f32 * 30. + 200.,
            y: 120.,
        });
    }
    loop {
        while let ControlFlow::Continue(()) = session.next_request(|request: Request<'_>| {
            match request {
                Request::SaveTo(vec) => bincode::serialize_into(vec, &game_state).unwrap(),
                Request::LoadFrom(buf) => {
                    match bincode::deserialize(buf) {
                        Ok(s) => game_state = s,
                        // Need to handle parsing errors as state could be from malicious peers.
                        Err(_) => {}
                    }
                }
                Request::CaptureLocalInput(vec) => {
                    let mut input = Vec2::default();

                    if is_key_down(KeyCode::Up) {
                        input.y -= 1.;
                    }
                    if is_key_down(KeyCode::Down) {
                        input.y += 1.;
                    }
                    if is_key_down(KeyCode::Left) {
                        input.x -= 1.;
                    }
                    if is_key_down(KeyCode::Right) {
                        input.x += 1.;
                    }

                    bincode::serialize_into(vec, &input).unwrap();
                }
                Request::Advance {
                    amount: dt, inputs, ..
                } => {
                    let inputs: PlayerInputs<Vec2> =
                        inputs.map(|vec| bincode::deserialize_from(&vec.into_inner()[..]).unwrap());
                    let speed = 100.;
                    for (player_id, input) in inputs.iter() {
                        let mut pos = game_state.box_positions.get_mut(player_id).unwrap();
                        pos.x += input.x * dt.as_secs_f32() * speed;
                        pos.y += input.y * dt.as_secs_f32() * speed;
                    }
                }
                unhandled => {
                    log::warn!("unhandled request: {:?}", unhandled);
                }
            }
        }) {}

        for pos in game_state.box_positions.values() {
            draw_rectangle(pos.x, pos.y, 10., 10., WHITE);
        }

        let network_stats = session.network_stats();
        let mut texts = vec![
            format!("Elapsed: {:?}", network_stats.elapsed),
            format!("Drift: {:?}", network_stats.drift),
        ];
        match network_stats.socket {
            Some(stats) => {
                texts.push(format!("Out: {:?}/s", stats.outgoing_bytes));
                texts.push(format!("In: {:?}/s", stats.incoming_bytes));
            }
            None => {}
        }
        for (i, text) in texts.into_iter().enumerate() {
            draw_text(&text, 0., 16. * (i + 1) as f32, 16., WHITE);
        }

        next_frame().await;
    }
}
