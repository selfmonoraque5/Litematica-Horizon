// The overlay hides behind terrain by the world's depth: keep it for `world_drawn`.
fn blazematica_render_needs(needs: &mut RenderNeeds) {
    if blazematica::overlay::wants_world_depth() {
        needs.world_depth = true;
    }
}
