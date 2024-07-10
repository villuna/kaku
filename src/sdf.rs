use std::cmp::Reverse;

use ahash::{HashSet, HashSetExt};
use image::{GrayImage, Luma};
use ordered_float::OrderedFloat;
use priority_queue::PriorityQueue;

/// Settings for how the signed distance field calculation should work for a font.
#[derive(Debug, Clone, Copy)]
pub struct SdfSettings {
    /// The sdf spread radius.
    ///
    /// This field defines the length of the distance field in pixels. This imposes a limit on the
    /// size of effects such as outlines, glow, shadows etc. A higher radius means you can create
    /// larger outlines, but will use more memory on the GPU.
    pub radius: f32,
    // Stuff to do in the future:

    // How much to scale up the texture when generating the sdf texture
    // A bigger scale will lead to higher quality glyphs that can be scaled up but will lead to
    // pub prescale: f32,
}

fn add_coords_checked(
    (w, h): (u32, u32),
    (x, y): (u32, u32),
    (dx, dy): (i64, i64),
) -> Option<(u32, u32)> {
    let x = x as i64 + dx;
    let y = y as i64 + dy;

    if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
        return None;
    }

    // We've checked x and y are >0 so this is fine
    let x = x as u32;
    let y = y as u32;

    Some((x, y))
}

/// Converts a coverage value from u8 to f32.
/// I'm using u8s to avoid floating point errors, but when I need to convert to float I will use
/// this.
fn value_u8_to_f32(value: u8) -> f32 {
    value as f32 / 255.
}

fn is_filled(value: u8) -> bool {
    value == 255 || value == 254
}

#[inline]
fn is_empty(value: u8) -> bool {
    value == 0
}

/// Calculates whether a point is on the boundary or not.
/// A point is a boundary point if the boundary crosses through it (i.e., 0 < coverage < 1), or it
/// is a fully covered pixel that borders a fully uncovered pixel.
fn is_boundary_point(image: &GrayImage, (w, h): (u32, u32), (x, y): (u32, u32)) -> bool {
    let value = image.get_pixel(x, y).0[0];

    if is_empty(value) {
        false
    } else if !is_filled(value) {
        true
    } else {
        // Check the surrounding vertices to see if there are any empty pixels
        for dx in -1..=1 {
            for dy in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }

                let Some((x, y)) = add_coords_checked((w, h), (x, y), (dx, dy)) else {
                    // This means we are on the boundary of the texture
                    // which is the boundary of the glyph, too!
                    return true;
                };
                let neighbouring_value = image.get_pixel(x, y).0[0];

                if is_empty(neighbouring_value) {
                    return true;
                }
            }
        }

        false
    }
}

/// This struct is private and used only for the function [create_sdf_textre].
/// it is a priority queue key used for a modified version of Dijkstra's algorithm.
struct PQKey {
    // The vector distance to the closest boundary point
    vector: [f32; 2],
    // The distance modifier of the closest boundary point (based on coverage)
    dist: f32,
    interior: bool,
}

impl PartialEq for PQKey {
    fn eq(&self, other: &Self) -> bool {
        OrderedFloat(self.vector[0]) == OrderedFloat(other.vector[0])
            && OrderedFloat(self.vector[1]) == OrderedFloat(other.vector[1])
            && OrderedFloat(self.dist) == OrderedFloat(other.dist)
    }
}

impl Eq for PQKey {}

impl PQKey {
    fn distance(&self) -> f32 {
        let mut vec_dist =
            (self.vector[0] * self.vector[0] + self.vector[1] * self.vector[1]).sqrt();

        if self.interior {
            vec_dist *= -1.0;
        }

        vec_dist + self.dist
    }
}

impl Ord for PQKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        OrderedFloat(self.distance().abs()).cmp(&OrderedFloat(other.distance().abs()))
    }
}

impl PartialOrd for PQKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) fn create_sdf_texture(
    image: &GrayImage,
    dimensions: (u32, u32),
    sdf: &SdfSettings,
) -> (GrayImage, u32) {
    // ab_glyph provides us with grayscale, anti-aliased images of glyphs. We can use this to our
    // advantage by using the value of an anti-aliased pixel to inform the distance calculation.

    // We take the ceiling to make sure there is enough space to accomodate the radius in the
    // worst case
    let texture_padding = sdf.radius.ceil() as u32;

    let new_dimensions = (
        dimensions.0 + 2 * texture_padding,
        dimensions.1 + 2 * texture_padding,
    );

    // converts the coordinates from the old texture to the new, expanded texture
    let convert_to_new_coord =
        |x: u32, y: u32| -> (u32, u32) { (x + texture_padding, y + texture_padding) };

    // converts the new coordinates to the old coordinates.
    // returns None if this pixel wouldn't have been in the old texture.
    let convert_to_old_coord = |x: u32, y: u32| -> Option<(u32, u32)> {
        let x = x.checked_sub(texture_padding)?;
        let y = y.checked_sub(texture_padding)?;

        (x < dimensions.0 && y < dimensions.1).then_some((x, y))
    };

    // converts the signed distance from an absolute float value to a scaled byte value for usage
    // in a texture.
    let convert_signed_dist =
        |val: f32| -> Luma<u8> { Luma([((val / (2. * sdf.radius) + 0.5) * 255.) as u8]) };

    let mut sdf_texture = GrayImage::new(new_dimensions.0, new_dimensions.1);

    // Use a modified dijkstra's algorithm, starting at the boundary pixels, to calculate the
    // distance from each pixel to its closest boundary

    let mut frontier = PriorityQueue::new();
    let mut visited = HashSet::new();

    for x in 0..new_dimensions.0 {
        for y in 0..new_dimensions.1 {
            sdf_texture.put_pixel(x, y, convert_signed_dist(sdf.radius));
        }
    }

    for x in 0..dimensions.0 {
        for y in 0..dimensions.1 {
            let (xp, yp) = convert_to_new_coord(x, y);

            if is_boundary_point(image, dimensions, (x, y)) {
                let signed_dist = 0.5 - value_u8_to_f32(image.get_pixel(x, y).0[0]);
                sdf_texture.put_pixel(xp, yp, convert_signed_dist(signed_dist));
                frontier.push(
                    (xp, yp),
                    Reverse(PQKey {
                        vector: [0., 0.],
                        dist: signed_dist,
                        interior: true,
                    }),
                );
                visited.insert((xp, yp));
            } else if is_filled(image.get_pixel(x, y).0[0]) {
                sdf_texture.put_pixel(xp, yp, convert_signed_dist(-sdf.radius));
            }
        }
    }

    while let Some(((x, y), Reverse(priority))) = frontier.pop() {
        sdf_texture.put_pixel(x, y, convert_signed_dist(priority.distance()));

        for dx in -1..=1 {
            for dy in -1..=1 {
                // Filtering out squares we don't want to visit
                // (this one, out of bounds, etc)
                if dx == 0 && dy == 0 {
                    continue;
                }

                let Some((x, y)) = add_coords_checked(new_dimensions, (x, y), (dx, dy)) else {
                    continue;
                };

                if visited.contains(&(x, y)) {
                    continue;
                }

                let interior = match convert_to_old_coord(x, y) {
                    Some((old_x, old_y)) => {
                        let value = image.get_pixel(old_x, old_y).0[0];
                        if is_empty(value) {
                            false
                        } else if is_filled(value) {
                            true
                        } else {
                            continue;
                        }
                    }
                    // Points that were not in the original texture are in the exterior
                    None => false,
                };

                let vector_distance = [dx as f32, dy as f32];

                let vector = [
                    vector_distance[0] + priority.vector[0],
                    vector_distance[1] + priority.vector[1],
                ];

                let new_key = PQKey {
                    vector,
                    dist: priority.dist,
                    interior,
                };

                if new_key.distance().abs() >= sdf.radius {
                    continue;
                }

                frontier.push_increase((x, y), Reverse(new_key));
            }
        }

        visited.insert((x, y));
    }

    (sdf_texture, texture_padding)
}
