# Kaku

A text rendering crate for rust+wgpu, with the ability to use signed distance fields. (It's still in a WIP state and is not on crates.io, yet).

This crate was originally designed with the intent of allowing fast frame-by-frame rendering of outlined text for a video game I'm working on.

## Features

- Rendering of OpenType fonts (loaded with [ab_glyph](https://github.com/alexheretic/ab-glyph)).
- Simple, non-SDF text rendering for performance.
- SDF-based text rendering for high quality upscaling and fast outlining.

## What do signed distance fields do?

Without going into details, signed distance fields are a way of representing a shape (such as a character in a font) in a way that allows for high quality upscaling, reducing memory usage. It also allows you to render certain effects such as outlines in a way that is very performant.

While SDF rendering has many benefits, it also takes a little bit longer to generate the textures for each character than with basic texture-based rendering. So this crate provides both methods of text rendering.

## Example

Here is a screenshot of the demo example, showing some of the things kaku can do:

![Example of kaku rendering text](images/demo.png)

This example shows the same text:

- Rendered with no SDF, using just the textures provided by ab_glyph.
- Rendered with SDF.
- Rendered with SDF, with a large outline.
- Rendered with SDF, upscaled 2x. Even though it's upscaled, it's still crisp and smooth!
