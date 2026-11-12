// The scene's haze, which every pipeline that draws in the world fades what it draws by: the environment's colour in
// `haze.rgb` and its density in `haze.w`, so that what is `distance` metres from the eye fades into the colour by
// `1 - exp(-density distance²)`. Each pipeline's shader is this and its own source after it; none fades with distance
// by its own. See `design/rendering.md`.

// How far what is `distance` metres from the eye has faded into `haze`'s colour, from 0 to 1.
fn haze_amount(haze: vec4<f32>, distance: f32) -> f32 {
    return 1.0 - exp(-haze.w * distance * distance);
}

// `color`, `distance` metres from the eye, seen through `haze`.
fn hazed(color: vec3<f32>, haze: vec4<f32>, distance: f32) -> vec3<f32> {
    return mix(color, haze.rgb, haze_amount(haze, distance));
}

// Light added to what is behind it, as a glow is, `distance` metres from the eye, seen through `haze`: as much of it as
// the haze lets through, the haze's own light being behind it already.
fn hazed_glow(light: vec3<f32>, haze: vec4<f32>, distance: f32) -> vec3<f32> {
    return light * (1.0 - haze_amount(haze, distance));
}
