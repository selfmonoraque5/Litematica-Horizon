// Once a client tick, before the window's tick: the overlay is built with the game's block
// models (see game_support.rs).
fn litematica_horizon_tick_pre(game: &mut crate::game::Game<'_>) {
    crate::game::litematica_horizon_game::tick(game);
}
