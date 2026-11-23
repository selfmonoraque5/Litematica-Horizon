// The overlay hides behind terrain by the world's depth: keep it for `world_drawn`.
fn litematica_horizon_render_needs(needs: &mut RenderNeeds) {
    if litematica_horizon::overlay::wants_world_depth() {
        needs.world_depth = true;
    }
}
