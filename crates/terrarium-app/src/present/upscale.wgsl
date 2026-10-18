// The strip's native image scaled by its integer, nearest pixel, with letterbox beside it and
// beyond the strip: pixel for pixel what `terrarium_compose::scale::upscale_into` writes.

struct Placement {
    // Physical pixels per native pixel.
    scale: u32,
    // Where the scaled image starts, and where it is cut: at its own end or the strip's edge.
    image_x: u32,
    image_end: u32,
    // The strip's height; the window can be taller while it settles.
    height: u32,
    // The letterbox colour, its bytes red first.
    letterbox: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var native: texture_2d<f32>;
@group(0) @binding(1) var<uniform> place: Placement;

// One triangle that covers the whole target.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = select(-1.0, 3.0, index == 1u);
    let y = select(-1.0, 3.0, index == 2u);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    // A fragment's position is its pixel's centre; truncating it gives the pixel.
    let pixel = vec2<u32>(at.xy);
    if pixel.x < place.image_x || pixel.x >= place.image_end || pixel.y >= place.height {
        return unpack4x8unorm(place.letterbox);
    }
    let source = vec2<u32>((pixel.x - place.image_x) / place.scale, pixel.y / place.scale);
    return textureLoad(native, source, 0);
}
