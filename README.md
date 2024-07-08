# Kaku

A text rendering crate for rust+wgpu, using signed distance fields for rendering.

This crate was originally designed with the intent of allowing fast frame-by-frame rendering of outlined text for a video game I'm working on. I intend to expand it to provide many of the features enabled by SDF rendering.

## Example

![Example of kaku rendering text](images/demo.png)

This example shows the same text:

- Rendered with no sdf, using just the textures provided by ab_glyph.
- Rendered with sdf.
- Rendered with sdf, with a large outline.
- Rendered with sdf, upscaled 2x (even though it's upscaled, it's still crisp and smooth!)
